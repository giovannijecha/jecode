use super::*;
use crate::config::{Settings, Store};
use crate::json::Value;
use crate::openrouter::OpenRouter;
use crate::test_support::{Directory, HttpFixture};
use crate::tools::Tools;
use std::io::Cursor;

fn response(text: &str) -> Value {
    Value::object([(
        "choices",
        Value::Array(vec![Value::object([
            ("finish_reason", Value::string("stop")),
            (
                "message",
                Value::object([
                    ("role", Value::string("assistant")),
                    ("content", Value::string(text)),
                ]),
            ),
        ])]),
    )])
}

#[test]
fn interactive_session_recovers_from_api_errors_and_can_clear_history() {
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![
        (
            401,
            Value::object([(
                "error",
                Value::object([("message", Value::string("Invalid fixture credential"))]),
            )]),
        ),
        (200, response("Recovered.")),
        (200, response("Fresh conversation.")),
    ]);
    let mut agent = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(directory.path()).unwrap(),
    );
    let mut input =
        Cursor::new("First question\nTry again\n/export\n/clear\nFresh question\n/export\n/exit\n");
    let mut output = Vec::new();
    let mut events = Vec::new();
    let mut config = SessionConfig {
        store: Store::new(directory.path().join(".jecode")),
        settings: Settings::new("isolated-fixture-key".into(), "fixture/model".into()).unwrap(),
        bash: crate::tools::find_bash().unwrap(),
    };
    chat(
        &mut agent,
        &mut config,
        &mut input,
        &mut output,
        &mut events,
    )
    .unwrap();
    let output = String::from_utf8(output).unwrap();
    assert!(output.contains("Recovered."));
    assert!(output.contains("Conversation cleared."));
    assert!(output.contains("Fresh conversation."));
    assert!(output.contains("Exported conversation to"));
    let exports: Vec<_> = std::fs::read_dir(directory.path())
        .unwrap()
        .flatten()
        .filter(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .starts_with("JECODE-SESSION-")
        })
        .map(|entry| std::fs::read_to_string(entry.path()).unwrap())
        .collect();
    assert_eq!(exports.len(), 2);
    assert!(
        exports
            .iter()
            .any(|text| text.contains("Recovered.") && !text.contains("Fresh conversation."))
    );
    assert!(
        exports
            .iter()
            .any(|text| text.contains("Fresh conversation.") && !text.contains("First question"))
    );
    assert!(String::from_utf8(events).unwrap().contains("HTTP 401"));
    let requests = fixture.finish();
    let messages = requests[2]
        .body
        .get("messages")
        .unwrap()
        .as_array()
        .unwrap();
    assert_eq!(messages.len(), 2);
    assert_eq!(
        messages[1].get("content").unwrap().as_str(),
        Some("Fresh question")
    );
}

#[test]
fn model_command_preserves_context_and_does_not_change_saved_defaults() {
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![
        (200, response("Before.")),
        (
            200,
            Value::object([(
                "data",
                Value::Array(vec![Value::object([
                    ("id", Value::string("fixture/new")),
                    (
                        "supported_parameters",
                        Value::Array(vec![Value::string("tools")]),
                    ),
                ])]),
            )]),
        ),
        (200, response("After.")),
    ]);
    let mut agent = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(directory.path()).unwrap(),
    );
    let mut config = SessionConfig {
        store: Store::new(directory.path().join(".jecode")),
        settings: Settings::new("isolated-fixture-key".into(), "fixture/model".into()).unwrap(),
        bash: crate::tools::find_bash().unwrap(),
    };
    let mut input =
        Cursor::new("First task\n/help\n/unknown\n/model fixture/new\nSecond task\n/exit\n");
    let mut output = Vec::new();
    chat(
        &mut agent,
        &mut config,
        &mut input,
        &mut output,
        &mut Vec::new(),
    )
    .unwrap();
    assert_eq!(agent.model(), "fixture/new");
    assert!(config.store.load().unwrap().is_none());
    let output = String::from_utf8(output).unwrap();
    assert!(output.contains("Unknown command"));
    assert!(output.contains("Model set to fixture/new"));
    let requests = fixture.finish();
    assert_eq!(
        requests[2].body.get("model").and_then(Value::as_str),
        Some("fixture/new")
    );
    assert_eq!(
        requests[2]
            .body
            .get("messages")
            .and_then(Value::as_array)
            .unwrap()
            .len(),
        4
    );
}
