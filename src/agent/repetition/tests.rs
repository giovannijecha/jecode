use super::*;
use crate::{
    agent::ToolCall,
    openrouter::OpenRouter,
    test_support::{Directory, HttpFixture, completion, tool_call},
    tools::Tools,
};

#[test]
fn archived_commands_need_a_review_after_resume_and_an_explicit_repeat_can_run() {
    let directory = Directory::new();
    let storage = Directory::new();
    let arguments = Value::object([
        (
            "command",
            Value::string("printf 'ran\\n' >> executions.txt"),
        ),
        ("check", Value::Bool(false)),
    ]);
    let fixture = HttpFixture::new(vec![
        (
            200,
            completion("", vec![tool_call("first", "bash", arguments.clone())]),
        ),
        (200, completion("Original operation performed", vec![])),
    ]);
    let mut first = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(directory.path()).unwrap(),
    );
    first.enable_sessions(storage.path()).unwrap();
    first
        .run_turn("Perform the marker operation once.", &mut |_| Ok(()))
        .unwrap();
    first.context.from = first.messages.lock().unwrap().len();
    first.context.summary = crate::context::memory::fixture("Continue the original work");
    first.save_session().unwrap();
    let id = first.sessions().unwrap().id();
    drop(first);
    assert_eq!(fixture.finish().len(), 2);
    let mut resumed = Agent::new(
        OpenRouter::fixture("http://127.0.0.1:1/unused".into()),
        Tools::new(directory.path()).unwrap(),
    );
    resumed.enable_sessions(storage.path()).unwrap();
    resumed.resume(&id).unwrap();
    let mut call = ToolCall {
        id: "repeat".into(),
        name: "bash".into(),
        arguments: arguments.encode(),
    };
    let refusal = resumed.execute_tool(&call);
    assert_eq!(
        refusal.get("outcome").and_then(Value::as_str),
        Some("not_started")
    );
    assert_eq!(
        refusal
            .get("previous_history_reference")
            .and_then(Value::as_str),
        Some("history:3")
    );
    assert!(refusal.get("check_status").is_none());
    assert_eq!(
        std::fs::read_to_string(directory.path().join("executions.txt")).unwrap(),
        "ran\n"
    );
    let mut repeat = arguments;
    if let Value::Object(fields) = &mut repeat {
        fields.insert(
            "repeat_reason".into(),
            Value::string("The new acceptance experiment requires a deliberate second execution"),
        );
    }
    call.arguments = repeat.encode();
    let result = resumed.execute_tool(&call);
    assert!(result.get("error").is_none(), "{}", result.encode());
    assert_eq!(
        std::fs::read_to_string(directory.path().join("executions.txt")).unwrap(),
        "ran\nran\n"
    );
}

fn previous(result: Value) -> (Directory, Agent, Value) {
    let directory = Directory::new();
    let mut agent = Agent::new(
        OpenRouter::fixture("http://127.0.0.1:1/unused".into()),
        Tools::new(directory.path()).unwrap(),
    );
    let arguments = Value::object([("command", Value::string("printf original"))]);
    agent.messages.lock().unwrap().extend([
        Value::object([
            ("role", Value::string("assistant")),
            (
                "tool_calls",
                Value::Array(vec![tool_call("old", "bash", arguments.clone())]),
            ),
        ]),
        Value::object([
            ("role", Value::string("tool")),
            ("tool_call_id", Value::string("old")),
            ("content", Value::string(result.encode())),
        ]),
    ]);
    agent.context.from = agent.messages.lock().unwrap().len();
    (directory, agent, arguments)
}

#[test]
fn visible_commands_and_refusals_are_not_mistaken_for_archived_execution() {
    let (_directory, mut agent, arguments) =
        previous(Value::object([("exit_code", Value::number(0))]));
    agent.context.from = 1;
    assert!(agent.repetition_refusal(&arguments).is_none());
    let (_directory, agent, arguments) = previous(Value::object([
        ("outcome", Value::string("not_started")),
        ("error", Value::string("Scope review required")),
    ]));
    assert!(agent.repetition_refusal(&arguments).is_none());
}

#[test]
fn unknown_outcomes_require_investigation_and_empty_repeat_reasons_cannot_bypass_review() {
    let (_directory, agent, mut arguments) =
        previous(Value::object([("outcome", Value::string("unknown"))]));
    assert!(agent.repetition_refusal(&arguments).is_some());
    if let Value::Object(fields) = &mut arguments {
        fields.insert("repeat_reason".into(), Value::string(" "));
    }
    assert!(agent.repetition_refusal(&arguments).is_some());
}

#[test]
fn contract_regression_empty_repeat_reason_does_not_block_a_first_command() {
    let directory = Directory::new();
    let agent = Agent::new(
        OpenRouter::fixture("http://127.0.0.1:1/unused".into()),
        Tools::new(directory.path()).unwrap(),
    );
    let call = ToolCall {
        id: "fresh".into(),
        name: "bash".into(),
        arguments: Value::object([
            ("command", Value::string("printf fresh")),
            ("check", Value::Bool(false)),
            ("repeat_reason", Value::string("")),
            ("watch", Value::Array(vec![])),
            ("timeout_seconds", Value::number(10)),
        ])
        .encode(),
    };
    let result = agent.execute_tool(&call);
    assert!(result.get("error").is_none(), "{}", result.encode());
    assert_eq!(result.get("stdout").and_then(Value::as_str), Some("fresh"));
}

#[test]
fn contract_regression_empty_repeat_reason_still_exposes_the_archived_result() {
    for result in [
        Value::object([("exit_code", Value::number(0))]),
        Value::object([("outcome", Value::string("unknown"))]),
    ] {
        let (_directory, agent, mut arguments) = previous(result);
        if let Value::Object(fields) = &mut arguments {
            fields.insert("repeat_reason".into(), Value::string(""));
        }
        let refusal = agent.repetition_refusal(&arguments).unwrap();
        assert_eq!(
            refusal
                .get("previous_history_reference")
                .and_then(Value::as_str),
            Some("history:2")
        );
        assert_eq!(
            refusal.get("outcome").and_then(Value::as_str),
            Some("not_started")
        );
    }
}
