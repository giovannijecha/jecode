use super::*;
use crate::{
    agent::Agent,
    test_support::{Directory, HttpFixture, Response},
    tools::Tools,
};
use std::{fs, time::Duration};

fn completed(output: Vec<Value>) -> Response {
    let event = Value::object([
        ("type", Value::string("response.completed")),
        (
            "response",
            Value::object([
                ("status", Value::string("completed")),
                ("output", Value::Array(output)),
            ]),
        ),
    ]);
    Response::Stream(vec![(
        Duration::ZERO,
        format!("data: {}\n\n", event.encode()),
    )])
}

fn client(fixture: &HttpFixture) -> Box<dyn Backend> {
    let api = Api::fixture(
        Session::fixture("isolated-access-token"),
        fixture.endpoint.trim_end_matches("/chat/completions"),
    );
    Box::new(OpenAi::with_api(api, "fixture-model".into()).unwrap())
}

#[test]
fn tool_execution_replays_reasoning_and_results_and_saves_the_same_conversation() {
    let project = Directory::new();
    let home = Directory::new();
    fs::write(project.path().join("source.txt"), "Original source content").unwrap();
    let reasoning = Value::object([
        ("type", Value::string("reasoning")),
        ("encrypted_content", Value::string("opaque-reasoning")),
    ]);
    let call = Value::object([
        ("type", Value::string("function_call")),
        ("namespace", Value::string("jecode")),
        ("call_id", Value::string("read-source")),
        ("name", Value::string("read")),
        ("arguments", Value::string(r#"{"path":"source.txt"}"#)),
    ]);
    let answer = Value::object([
        ("type", Value::string("message")),
        ("role", Value::string("assistant")),
        ("phase", Value::string("final_answer")),
        (
            "content",
            Value::Array(vec![Value::object([
                ("type", Value::string("output_text")),
                ("text", Value::string("Source reviewed.")),
            ])]),
        ),
    ]);
    let fixture = HttpFixture::streaming(vec![
        completed(vec![reasoning, call]),
        completed(vec![answer]),
    ]);
    let mut agent = Agent::with_backend(client(&fixture), Tools::new(project.path()).unwrap());
    agent.enable_sessions(home.path()).unwrap();
    agent
        .run_turn("Read source.txt and review it", &mut |_| Ok(()))
        .unwrap();
    let requests = fixture.finish();
    assert_eq!(requests.len(), 2);
    assert!(
        requests
            .iter()
            .all(|request| request.headers[0].starts_with("POST /responses "))
    );
    let input = requests[1].body.get("input").unwrap().as_array().unwrap();
    let reasoning = input
        .iter()
        .position(|item| item.get("type").and_then(Value::as_str) == Some("reasoning"))
        .unwrap();
    let output = input
        .iter()
        .position(|item| item.get("type").and_then(Value::as_str) == Some("function_call_output"))
        .unwrap();
    assert!(reasoning < output);
    assert_eq!(
        input[reasoning]
            .get("encrypted_content")
            .and_then(Value::as_str),
        Some("opaque-reasoning")
    );
    assert!(
        input[output]
            .get("output")
            .and_then(Value::as_str)
            .unwrap()
            .contains("Original source content")
    );
    assert!(!requests[1].body.encode().contains("jecode_origin"));
    let handle = agent.sessions().unwrap();
    handle.flush().unwrap();
    let document = handle.snapshot();
    assert_eq!(document.backend, "openai");
    let last = document.messages.last().unwrap();
    assert_eq!(
        last.get("content").and_then(Value::as_str),
        Some("Source reviewed.")
    );
    assert!(
        last.get("jecode_origin")
            .and_then(|origin| origin.get("account"))
            .is_some()
    );
    assert!(
        !agent
            .archive()
            .document()
            .encode()
            .contains("isolated-access-token")
    );
}

#[test]
fn usage_limit_after_a_streamed_call_keeps_files_and_history_without_executing_it() {
    let project = Directory::new();
    let fixture = HttpFixture::streaming(vec![Response::Stream(vec![(Duration::ZERO, concat!(
        "data: {\"type\":\"response.output_item.done\",\"output_index\":0,\"item\":{\"type\":\"function_call\",\"namespace\":\"jecode\",\"call_id\":\"never-write\",\"name\":\"write\",\"arguments\":\"{\\\"path\\\":\\\"untouched.txt\\\",\\\"content\\\":\\\"must not run\\\"}\"}}\n\n",
        "data: {\"type\":\"response.failed\",\"response\":{\"error\":{\"code\":\"subscription_sharing_usage_limit_exceeded\",\"message\":\"app limit\"}}}\n\n"
    ).into())])]);
    let mut agent = Agent::with_backend(client(&fixture), Tools::new(project.path()).unwrap());
    let error = agent
        .run_turn("Review this task", &mut |_| Ok(()))
        .unwrap_err();
    assert!(error.contains("subscription_sharing_usage_limit_exceeded"));
    assert!(!project.path().join("untouched.txt").exists());
    assert!(
        agent
            .archive()
            .document()
            .encode()
            .contains("Review this task")
    );
    assert_eq!(fixture.finish().len(), 1);
}
