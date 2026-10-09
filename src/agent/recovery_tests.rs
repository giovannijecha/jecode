use super::*;
use crate::test_support::{Directory, HttpFixture, completion, tool_call};

#[test]
fn an_empty_completion_is_retried_without_replaying_a_completed_tool() {
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![
        (
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
        ),
        (200, completion("", vec![])),
        (200, completion("Recovered.", vec![])),
    ]);
    let mut agent = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(directory.path()).unwrap(),
    );
    agent.run_turn("Execute once", &mut |_| Ok(())).unwrap();
    assert_eq!(
        std::fs::read_to_string(directory.path().join("executions.txt")).unwrap(),
        "once\n"
    );
    let requests = fixture.finish();
    assert_eq!(requests.len(), 3);
    assert_eq!(
        requests[1].body.get("messages"),
        requests[2].body.get("messages")
    );
}

#[test]
fn an_empty_summary_response_recovers_before_any_tools_can_execute() {
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![
        (200, completion("", vec![])),
        (200, completion("Keep the goal and next action.", vec![])),
    ]);
    let agent = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(directory.path()).unwrap(),
    );
    let messages = [Value::object([
        ("role", Value::string("user")),
        ("content", Value::string("Summarize")),
    ])];
    let result = agent
        .complete_retry(&messages, false, Some(3000), &mut |_| Ok(()))
        .unwrap();
    assert!(result.calls.is_empty());
    assert_eq!(result.text, "Keep the goal and next action.");
    let requests = fixture.finish();
    assert_eq!(requests.len(), 2);
    assert!(
        requests
            .iter()
            .all(|request| request.body.get("tools").is_none())
    );
    assert!(
        agent
            .events
            .lock()
            .unwrap()
            .iter()
            .any(|event| event.get("operation").and_then(Value::as_str)
                == Some("context_compaction"))
    );
}

#[test]
fn repeated_empty_responses_stop_with_the_original_request_preserved() {
    let directory = Directory::new();
    let fixture = HttpFixture::new((0..3).map(|_| (200, completion("", vec![]))).collect());
    let mut agent = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(directory.path()).unwrap(),
    );
    let error = agent
        .run_turn("Keep this exact goal", &mut |_| Ok(()))
        .unwrap_err();
    assert!(error.contains("empty"));
    assert!(error.contains("preserved"));
    assert_eq!(agent.messages.lock().unwrap().len(), 2);
    assert_eq!(
        agent.messages.lock().unwrap()[1]
            .get("content")
            .and_then(Value::as_str),
        Some("Keep this exact goal")
    );
    assert_eq!(fixture.finish().len(), 3);
}

#[test]
fn provider_reported_empty_responses_use_the_same_bounded_recovery() {
    let directory = Directory::new();
    let empty = Value::object([(
        "error",
        Value::object([
            (
                "message",
                Value::string("Provider returned an empty response"),
            ),
            ("code", Value::number(502)),
        ]),
    )]);
    let fixture = HttpFixture::new((0..3).map(|_| (502, empty.clone())).collect());
    let mut agent = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(directory.path()).unwrap(),
    );
    let error = agent
        .run_turn("Keep the goal", &mut |_| Ok(()))
        .unwrap_err();
    assert!(error.contains("three empty responses"));
    assert_eq!(agent.messages.lock().unwrap().len(), 2);
    assert_eq!(fixture.finish().len(), 3);
}
