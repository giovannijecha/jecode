use super::*;
use crate::test_support::{Directory, HttpFixture, Response, completion, tool_call};
use std::time::{Duration, Instant};

fn failure(code: usize, message: &str) -> Value {
    Value::object([(
        "error",
        Value::object([
            ("code", Value::number(code)),
            ("message", Value::string(message)),
        ]),
    )])
}

fn used(mut reply: Value, input: usize) -> Value {
    if let Value::Object(fields) = &mut reply {
        fields.insert(
            "usage".into(),
            Value::object([
                ("prompt_tokens", Value::number(input)),
                ("completion_tokens", Value::number(50)),
            ]),
        );
    }
    reply
}

fn fixture_agent(
    directory: &Directory,
    home: &Directory,
    endpoint: &str,
    capacity: usize,
) -> Agent {
    let mut client = OpenRouter::fixture(endpoint.into());
    client.fixture_limits(capacity, Some(4096));
    let mut agent = Agent::new(client, Tools::new(directory.path()).unwrap());
    agent.enable_sessions(home.path()).unwrap();
    agent
}

fn compacting_agent(directory: &Directory, home: &Directory, endpoint: &str) -> Agent {
    let mut agent = fixture_agent(directory, home, endpoint, usize::MAX);
    let capacity = agent.fixture_capacity(6_000, crate::context::AUTO_COMPACT_DEFAULT);
    agent.client.fixture_limits(capacity, Some(4096));
    agent
}

#[test]
fn temporary_failures_have_no_attempt_cap_and_completed_tools_are_not_replayed() {
    let directory = Directory::new();
    let home = Directory::new();
    let mut replies = vec![(
        200,
        completion(
            "",
            vec![tool_call(
                "once",
                "bash",
                Value::object([(
                    "command",
                    Value::string("printf 'once\\n' >> executions.txt"),
                )]),
            )],
        ),
    )];
    replies.extend((0..55).map(|_| (429, failure(429, "Try later isolated-fixture-key"))));
    replies.push((503, failure(503, "Provider unavailable")));
    replies.push((200, completion("Recovered and finished.", vec![])));
    let fixture = HttpFixture::new(replies);
    let mut agent = fixture_agent(&directory, &home, &fixture.endpoint, usize::MAX);
    agent
        .permissions()
        .set_level(crate::permissions::Level::Full)
        .unwrap();
    let mut retries = vec![];
    agent
        .run_turn("Run once and finish", &mut |event| {
            if let Event::Recovering { attempt, error, .. } = event {
                retries.push(attempt);
                assert!(!error.contains("isolated-fixture-key"));
            }
            Ok(())
        })
        .unwrap();
    assert_eq!(
        std::fs::read_to_string(directory.path().join("executions.txt")).unwrap(),
        "once\n"
    );
    assert_eq!(retries, (1..=56).collect::<Vec<_>>());
    let requests = fixture.finish();
    assert_eq!(requests.len(), 58);
    assert!(
        requests[1..]
            .iter()
            .all(|request| request.body.get("messages") == requests[1].body.get("messages"))
    );
    let doc = agent
        .sessions()
        .unwrap()
        .store()
        .fixture_load(&agent.sessions().unwrap().id())
        .unwrap();
    assert_eq!(
        doc.events
            .iter()
            .filter(|event| event.get("type").and_then(Value::as_str) == Some("request_recovery"))
            .count(),
        56
    );
    let timings: Vec<_> = doc
        .events
        .iter()
        .filter(|event| event.get("type").and_then(Value::as_str) == Some("request_timing"))
        .collect();
    assert_eq!(timings.len(), 58);
    assert_eq!(
        timings
            .iter()
            .filter(|event| event.get("status").and_then(Value::as_str) == Some("failed"))
            .count(),
        56
    );
    assert_eq!(
        timings
            .last()
            .unwrap()
            .get("attempt")
            .and_then(Value::as_usize),
        Some(57)
    );
    assert!(!doc.value().encode().contains("isolated-fixture-key"));
}

#[test]
fn retry_after_is_respected_and_user_interruption_breaks_the_wait_immediately() {
    for seconds in [90, u64::MAX] {
        let directory = Directory::new();
        let fixture = HttpFixture::streaming(vec![Response::Http(
            429,
            vec![("Retry-After".into(), seconds.to_string())],
            failure(429, "Wait"),
        )]);
        let mut agent = Agent::new(
            OpenRouter::fixture(fixture.endpoint.clone()),
            Tools::new(directory.path()).unwrap(),
        );
        let signal = agent.cancellation.clone();
        let started = Instant::now();
        let error = agent
            .run_turn("Work", &mut |event| {
                if let Event::Recovering { delay, .. } = event {
                    assert_eq!(delay, Duration::from_secs(seconds));
                    signal.cancel();
                }
                Ok(())
            })
            .unwrap_err();
        assert!(error.contains("cancelled"));
        assert!(started.elapsed() < Duration::from_secs(2));
        assert_eq!(fixture.finish().len(), 1);
    }
}

#[test]
fn authentication_failure_returns_to_the_user_without_retrying() {
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![(401, failure(401, "Invalid key"))]);
    let mut agent = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(directory.path()).unwrap(),
    );
    let error = agent
        .run_turn("Work", &mut |event| {
            assert!(!matches!(event, Event::Recovering { .. }));
            Ok(())
        })
        .unwrap_err();
    assert!(error.contains("401"));
    assert_eq!(fixture.finish().len(), 1);
}

#[test]
fn streamed_tool_batches_are_not_limited_to_sixteen_calls() {
    let directory = Directory::new();
    let calls = (0..24)
        .map(|index| {
            let Value::Object(mut fields) = tool_call(
                &format!("read-{index}"),
                "read",
                Value::object([("path", Value::string("missing"))]),
            ) else {
                unreachable!()
            };
            fields.insert("index".into(), Value::number(index));
            Value::Object(fields)
        })
        .collect();
    let chunk = Value::object([(
        "choices",
        Value::Array(vec![Value::object([
            (
                "delta",
                Value::object([("tool_calls", Value::Array(calls))]),
            ),
            ("finish_reason", Value::string("tool_calls")),
        ])]),
    )]);
    let fixture = HttpFixture::streaming(vec![
        Response::Stream(vec![(
            Duration::ZERO,
            format!("data: {}\n\ndata: [DONE]\n\n", chunk.encode()),
        )]),
        Response::Json(200, completion("All calls resolved.", vec![])),
    ]);
    let mut agent = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(directory.path()).unwrap(),
    );
    let mut finished = 0;
    agent
        .run_turn("Inspect", &mut |event| {
            if matches!(event, Event::ToolFinished { .. }) {
                finished += 1;
            }
            Ok(())
        })
        .unwrap();
    assert_eq!(finished, 24);
    let requests = fixture.finish();
    assert_eq!(
        requests[1]
            .body
            .get("messages")
            .unwrap()
            .as_array()
            .unwrap()
            .iter()
            .filter(|message| message.get("role").and_then(Value::as_str) == Some("tool"))
            .count(),
        24
    );
}

#[test]
fn an_interrupted_stream_is_archived_and_its_incomplete_tool_call_never_executes() {
    let directory = Directory::new();
    let chunk = "data: {\"choices\":[{\"delta\":{\"content\":\"Partial answer\",\"tool_calls\":[{\"index\":0,\"id\":\"partial\",\"function\":{\"name\":\"write\",\"arguments\":\"{\\\"path\\\":\\\"must-not-exist\\\",\\\"content\\\":\"}}]},\"finish_reason\":null}]}\n\ndata: {\"error\":{\"code\":\"server_error\",\"message\":\"Temporary outage\"}}\n\n";
    let fixture = HttpFixture::streaming(vec![
        Response::Stream(vec![(Duration::ZERO, chunk.into())]),
        Response::Json(200, completion("Complete answer", vec![])),
    ]);
    let mut agent = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(directory.path()).unwrap(),
    );
    agent.run_turn("Work", &mut |_| Ok(())).unwrap();
    assert!(!directory.path().join("must-not-exist").exists());
    assert!(
        agent
            .archive()
            .document()
            .encode()
            .contains("Partial answer")
    );
    assert_eq!(agent.messages.lock().unwrap().len(), 3);
    assert_eq!(fixture.finish().len(), 2);
}

#[test]
fn repeated_previews_keep_original_history_and_save_context_before_continuing() {
    let directory = Directory::new();
    let home = Directory::new();
    let original = "source evidence\n".repeat(1600);
    std::fs::write(directory.path().join("source"), &original).unwrap();
    // Budget-relative synthetic usage keeps each compaction at the post-read boundary.
    let read = |id| {
        used(
            completion(
                "",
                vec![tool_call(
                    id,
                    "read",
                    Value::object([
                        ("path", Value::string("source")),
                        ("limit", Value::number(2000)),
                    ]),
                )],
            ),
            crate::context::Context::default().estimate_bytes(6000 + crate::tools::encoded_size()),
        )
    };
    let fixture = HttpFixture::new(vec![
        (200, read("first")),
        (200, read("second")),
        (200, completion("Inspection complete.", vec![])),
    ]);
    let mut agent = compacting_agent(&directory, &home, &fixture.endpoint);
    let handle = agent.sessions().unwrap();
    let mut previews = 0;
    agent
        .run_turn("Inspect the source and report evidence", &mut |event| {
            if matches!(event, Event::ContextCompacted { ref text } if text.starts_with("Large tool results")) {
                previews += 1;
                let saved = handle.store().fixture_load(&handle.id()).unwrap();
                assert!(saved.context.preview_until >= 4);
                assert!(saved.pending.active);
            }
            Ok(())
        })
        .unwrap();
    assert_eq!(previews, 2);
    let requests = fixture.finish();
    for request in &requests {
        assert!(request.body.get("tools").is_some());
        assert_eq!(
            request.body.get("model").and_then(Value::as_str),
            Some("fixture/model")
        );
    }
    assert_eq!(requests.len(), 3);
    assert!(requests[1].body.encode().contains("history:3"));
    assert!(requests[2].body.encode().contains("history:5"));
    let doc = handle.store().fixture_load(&handle.id()).unwrap();
    assert_eq!(doc.messages.len(), 7);
    let saved_from = doc.context.from;
    assert_eq!(saved_from, 1);
    assert_eq!(doc.context.preview_until, 6);
    assert!(
        doc.messages[3]
            .get("content")
            .unwrap()
            .as_str()
            .unwrap()
            .contains("source evidence")
    );
    let id = handle.id();
    drop(handle);
    drop(agent);
    let mut resumed = compacting_agent(&directory, &home, "http://127.0.0.1:1/chat/completions");
    resumed.resume(&id).unwrap();
    assert_eq!(resumed.context.from, saved_from);
    assert!(
        resumed
            .transport_messages()
            .iter()
            .any(|message| message.encode().contains("history:5"))
    );
    assert!(
        resumed
            .archive()
            .document()
            .encode()
            .contains("source evidence")
    );
}

#[test]
fn cancelling_compaction_keeps_the_previous_context_and_original_tool_results() {
    let directory = Directory::new();
    let home = Directory::new();
    let fixture = HttpFixture::new(vec![]);
    let mut agent = compacting_agent(&directory, &home, &fixture.endpoint);
    agent.prepare_turn("Inspect").unwrap();
    let original_evidence = "original evidence\n".repeat(1800);
    agent.messages.lock().unwrap().extend([
        Value::object([
            ("role", Value::string("assistant")),
            (
                "tool_calls",
                Value::Array(vec![tool_call(
                    "read",
                    "read",
                    Value::object([("path", Value::string("source"))]),
                )]),
            ),
        ]),
        Value::object([
            ("role", Value::string("tool")),
            ("tool_call_id", Value::string("read")),
            (
                "content",
                Value::string(
                    Value::object([("content", Value::string(&original_evidence))]).encode(),
                ),
            ),
        ]),
        Value::object([
            ("role", Value::string("assistant")),
            ("content", Value::string("Earlier analysis. ".repeat(7_000))),
        ]),
        Value::object([
            ("role", Value::string("user")),
            ("content", Value::string("Finish the inspection")),
        ]),
    ]);
    let original_context = agent.context.value();
    let original_messages = agent.messages.lock().unwrap().clone();
    let signal = agent.cancellation.clone();
    let capacity = agent.fixture_capacity(40_000, 80);
    let error = agent
        .next_completion(
            Some(crate::backend::Limits {
                context: capacity,
                output: Some(4096),
            }),
            &mut |event| {
                if matches!(event, Event::Maintenance { .. }) {
                    signal.cancel();
                }
                Ok(())
            },
        )
        .err()
        .expect("cancellation must stop compaction");
    assert!(error.contains("cancelled"), "{error}");
    assert_eq!(agent.context.value(), original_context);
    assert_eq!(*agent.messages.lock().unwrap(), original_messages);
    assert_eq!(fixture.finish().len(), 0);
}
