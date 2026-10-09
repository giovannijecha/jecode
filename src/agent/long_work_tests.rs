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
    assert!(!doc.value().encode().contains("isolated-fixture-key"));
}

#[test]
fn retry_after_is_respected_and_user_interruption_breaks_the_wait_immediately() {
    let directory = Directory::new();
    let fixture = HttpFixture::streaming(vec![Response::Http(
        429,
        vec![("Retry-After".into(), "90".into())],
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
                assert_eq!(delay, Duration::from_secs(90));
                signal.cancel();
            }
            Ok(())
        })
        .unwrap_err();
    assert!(error.contains("cancelled"));
    assert!(started.elapsed() < Duration::from_secs(2));
    assert_eq!(fixture.finish().len(), 1);
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
fn repeated_compaction_keeps_original_history_and_saves_memory_before_continuing() {
    let directory = Directory::new();
    let home = Directory::new();
    let original = "source evidence\n".repeat(800);
    std::fs::write(directory.path().join("source"), &original).unwrap();
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
            6000,
        )
    };
    let fixture = HttpFixture::new(vec![
        (200, read("first")),
        (
            200,
            completion(
                &String::from("Goal: inspect source. First read completed; continue verification."),
                vec![],
            ),
        ),
        (200, read("second")),
        (
            200,
            completion(
                &String::from("Goal: inspect source. Both reads completed; report verified facts."),
                vec![],
            ),
        ),
        (200, completion("Inspection complete.", vec![])),
    ]);
    let mut agent = fixture_agent(&directory, &home, &fixture.endpoint, 24000);
    let handle = agent.sessions().unwrap();
    let mut compacted = 0;
    agent
        .run_turn("Inspect the source and report evidence", &mut |event| {
            if matches!(event, Event::ContextCompacted { .. }) {
                compacted += 1;
                let saved = handle.store().fixture_load(&handle.id()).unwrap();
                assert!(!saved.context.summary.is_empty());
                assert!(saved.pending.active);
            }
            Ok(())
        })
        .unwrap();
    assert_eq!(compacted, 2);
    let requests = fixture.finish();
    for index in [1, 3] {
        assert!(requests[index].body.get("tools").is_none());
        assert_eq!(
            requests[index].body.get("model").and_then(Value::as_str),
            Some("fixture/model")
        );
    }
    assert!(requests[3].body.encode().contains("First read completed"));
    assert!(requests[4].body.encode().contains("Both reads completed"));
    let doc = handle.store().fixture_load(&handle.id()).unwrap();
    assert_eq!(doc.messages.len(), 7);
    let saved_from = doc.context.from;
    assert!(saved_from > 1);
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
    let mut resumed = fixture_agent(
        &directory,
        &home,
        "http://127.0.0.1:1/chat/completions",
        24000,
    );
    resumed.resume(&id).unwrap();
    assert_eq!(resumed.context.from, saved_from);
    assert!(
        resumed
            .transport_messages()
            .iter()
            .any(|message| message.encode().contains("Both reads completed"))
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
    std::fs::write(
        directory.path().join("source"),
        "original evidence\n".repeat(900),
    )
    .unwrap();
    let fixture = HttpFixture::new(vec![(
        200,
        used(
            completion(
                "",
                vec![tool_call(
                    "read",
                    "read",
                    Value::object([
                        ("path", Value::string("source")),
                        ("limit", Value::number(2000)),
                    ]),
                )],
            ),
            6000,
        ),
    )]);
    let mut agent = fixture_agent(&directory, &home, &fixture.endpoint, 24000);
    let signal = agent.cancellation.clone();
    assert!(
        agent
            .run_turn("Inspect", &mut |event| {
                if matches!(event, Event::Maintenance { .. }) {
                    signal.cancel();
                }
                Ok(())
            })
            .unwrap_err()
            .contains("cancelled")
    );
    assert_eq!(agent.context.from, 1);
    assert!(agent.context.summary.is_empty());
    assert!(
        agent.messages.lock().unwrap()[3]
            .encode()
            .contains("original evidence")
    );
    assert_eq!(fixture.finish().len(), 1);
}
