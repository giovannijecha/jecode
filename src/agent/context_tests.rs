use super::*;
use crate::test_support::{Directory, HttpFixture, Response, completion, tool_call};
use std::time::Duration;

fn text(messages: &[Value]) -> String {
    messages
        .iter()
        .filter_map(|message| message.get("content").and_then(Value::as_str))
        .collect()
}

#[test]
fn current_request_boundary_survives_compaction_resume_and_a_new_goal() {
    let directory = Directory::new();
    let home = Directory::new();
    let fixture = HttpFixture::new(vec![]);
    let make_agent = || {
        Agent::new(
            OpenRouter::fixture(fixture.endpoint.clone()),
            Tools::new(directory.path()).unwrap(),
        )
    };
    let mut agent = make_agent();
    for (role, text) in [
        ("user", "Earlier implementation request"),
        ("assistant", "Earlier implementation finished"),
        ("user", "Change the current goal"),
        (
            "assistant",
            "Current work summarized into continuity memory",
        ),
    ] {
        agent.messages.lock().unwrap().push(Value::object([
            ("role", Value::string(role)),
            ("content", Value::string(text)),
        ]));
    }
    agent.context.from = 5;
    agent.context.summary = String::from("Change the current goal");
    let original = agent.messages.lock().unwrap().clone();
    {
        // Compaction estimates an already locked history; instructions borrow it.
        let history = agent.messages.lock().unwrap();
        assert!(agent.context_estimate(&history) < 21000);
    }
    assert!(
        text(&agent.transport_messages()).contains("history:3 — current original user request")
    );
    assert_eq!(*agent.messages.lock().unwrap(), original);
    agent.enable_sessions(home.path()).unwrap();
    let id = agent.sessions().unwrap().id();
    drop(agent);
    let mut resumed = make_agent();
    resumed.enable_sessions(home.path()).unwrap();
    resumed.resume(&id).unwrap();
    assert_eq!(*resumed.messages.lock().unwrap(), original);
    assert!(
        text(&resumed.transport_messages()).contains("history:3 — current original user request")
    );
    resumed
        .prepare_turn("Audit without more code changes")
        .unwrap();
    let payload = text(&resumed.transport_messages());
    assert!(payload.contains("history:3 — archived user request"));
    assert!(payload.ends_with("Audit without more code changes"));
    assert!(fixture.finish().is_empty());
}

fn delta(fields: Value, finish: Value) -> String {
    format!(
        "data: {}\n\n",
        Value::object([(
            "choices",
            Value::Array(vec![Value::object([
                ("index", Value::number(0)),
                ("delta", fields),
                ("finish_reason", finish),
            ])])
        )])
        .encode()
    )
}

#[test]
fn live_curl_delivers_reasoning_and_text_before_completion_and_tools_wait_for_valid_finish() {
    let directory = Directory::new();
    let detail = Value::object([
        ("type", Value::string("reasoning.encrypted")),
        ("data", Value::string("opaque")),
    ]);
    let first = delta(
        Value::object([("reasoning_details", Value::Array(vec![detail.clone()]))]),
        Value::Null,
    );
    let text = delta(
        Value::object([("content", Value::string("Inspecting."))]),
        Value::Null,
    );
    let call = delta(
        Value::object([(
            "tool_calls",
            Value::Array(vec![Value::object([
                ("index", Value::number(0)),
                ("id", Value::string("write-1")),
                ("type", Value::string("function")),
                (
                    "function",
                    Value::object([
                        ("name", Value::string("write")),
                        (
                            "arguments",
                            Value::string("{\"path\":\"file\",\"content\":\"done\"}"),
                        ),
                    ]),
                ),
            ])]),
        )]),
        Value::Null,
    );
    let finish = delta(Value::object([]), Value::string("tool_calls")) + "data: [DONE]\n\n";
    let (release, receive) = std::sync::mpsc::channel();
    let mut release = Some(release);
    let fixture = HttpFixture::streaming(vec![
        Response::GatedStream {
            head: first + &text + &call,
            tail: finish,
            release: receive,
        },
        Response::Json(200, completion("Done.", vec![])),
    ]);
    let mut agent = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(directory.path()).unwrap(),
    );
    let mut streamed = false;
    let mut thinking = false;
    agent
        .run_turn("create file", &mut |event| {
            match event {
                Event::Reasoning => thinking = true,
                Event::Streaming { .. } => {
                    streamed = true;
                    assert!(thinking);
                    assert!(!directory.path().join("file").exists());
                    if let Some(release) = release.take() {
                        release.send(()).unwrap();
                    }
                }
                Event::Working => assert!(!directory.path().join("file").exists()),
                Event::Recovering { .. } => return Err("Fixture stream did not complete".into()),
                _ => {}
            };
            Ok(())
        })
        .unwrap();
    assert!(thinking);
    assert!(streamed);
    assert_eq!(
        std::fs::read_to_string(directory.path().join("file")).unwrap(),
        "done"
    );
    let requests = fixture.finish();
    assert_eq!(requests[0].body.get("stream"), Some(&Value::Bool(true)));
    let messages = requests[1]
        .body
        .get("messages")
        .unwrap()
        .as_array()
        .unwrap();
    assert_eq!(
        messages[2]
            .get("reasoning_details")
            .unwrap()
            .as_array()
            .unwrap(),
        &[detail]
    );
}

#[test]
fn model_boundaries_keep_tool_pairs_and_archive_evidence_but_strip_old_provider_fields() {
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![
        (
            200,
            completion(
                "",
                vec![tool_call(
                    "read-1",
                    "read",
                    Value::object([("path", Value::string("missing"))]),
                )],
            ),
        ),
        (200, completion("Before", vec![])),
        (200, completion("After", vec![])),
    ]);
    let mut agent = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(directory.path()).unwrap(),
    );
    agent.run_turn("first", &mut |_| Ok(())).unwrap();
    agent.set_model("fixture/other".into()).unwrap();
    agent.run_turn("second", &mut |_| Ok(())).unwrap();
    let requests = fixture.finish();
    let messages = requests[2]
        .body
        .get("messages")
        .unwrap()
        .as_array()
        .unwrap();
    assert_eq!(messages.len(), 6);
    assert!(messages[2].get("reasoning_details").is_none());
    assert!(messages[4].get("reasoning_details").is_none());
    assert_eq!(
        messages[3].get("tool_call_id").and_then(Value::as_str),
        Some("read-1")
    );
    assert!(
        agent
            .archive()
            .document()
            .encode()
            .contains("fixture-reasoning")
    );
}

#[test]
fn a_failed_stream_exports_partial_text_as_evidence_and_does_not_execute_partial_calls() {
    let directory = Directory::new();
    let text = delta(
        Value::object([("content", Value::string("Partial answer"))]),
        Value::Null,
    );
    let fixture = HttpFixture::streaming(vec![Response::Stream(vec![(Duration::ZERO, text)])]);
    let mut agent = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(directory.path()).unwrap(),
    );
    assert!(
        agent
            .run_turn("question", &mut |event| {
                if matches!(event, Event::Recovering { .. }) {
                    Err("fixture stops recovery".into())
                } else {
                    Ok(())
                }
            })
            .is_err()
    );
    let document = agent.archive().document();
    let events = document.get("events").unwrap().encode();
    assert!(events.contains("Partial answer"));
    assert!(events.contains("turn_error"));
    assert_eq!(
        document.get("messages").unwrap().as_array().unwrap().len(),
        2
    );
    fixture.finish();
}

#[test]
fn replacing_a_key_keeps_all_prior_credentials_redacted_in_the_archive() {
    let directory = Directory::new();
    let mut agent = Agent::new(
        OpenRouter::fixture("http://127.0.0.1:1/chat/completions".into()),
        Tools::new(directory.path()).unwrap(),
    );
    agent.messages.lock().unwrap().push(Value::object([(
        "content",
        Value::string("isolated-fixture-key and replacement-fixture-key"),
    )]));
    let client = OpenRouter::with_api(
        agent
            .api()
            .with_key("replacement-fixture-key".into())
            .unwrap(),
        agent.model().into(),
    )
    .unwrap();
    agent.replace_client(client);
    let exported = agent.archive().document().encode();
    assert!(!exported.contains("isolated-fixture-key"));
    assert!(!exported.contains("replacement-fixture-key"));
    assert_eq!(agent.messages.lock().unwrap().len(), 2);
}
