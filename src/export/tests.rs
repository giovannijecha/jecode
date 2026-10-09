use super::*;
use crate::agent::Agent;
use crate::json;
use crate::openrouter::OpenRouter;
use crate::test_support::Directory;
use crate::tools::Tools;

#[test]
fn exports_exact_protocol_messages_and_masks_the_key_in_nested_arguments() {
    let directory = Directory::new();
    let arguments = Value::object([("command", Value::string("echo fixture-secret"))]);
    let original = vec![
        Value::object([
            ("role", Value::string("assistant")),
            (
                "reasoning_details",
                Value::Array(vec![Value::string("opaque reasoning")]),
            ),
            (
                "tool_calls",
                Value::Array(vec![Value::object([
                    ("id", Value::string("call-1")),
                    ("arguments", Value::string(arguments.encode())),
                ])]),
            ),
        ]),
        Value::object([
            ("role", Value::string("tool")),
            ("tool_call_id", Value::string("call-1")),
            (
                "content",
                Value::string("{\"stdout\":\"hello\\n\",\"truncated\":true}"),
            ),
        ]),
    ];
    let archive = Archive {
        effort: "default".into(),
        events: Arc::new(Mutex::new(vec![])),
        model: "fixture/model".into(),
        directory: directory.path().to_path_buf(),
        messages: Arc::new(Mutex::new(original.clone())),
        redactor: Redactor::new("fixture-secret".into()),
    };
    let first = archive.save().unwrap();
    let second = archive.save().unwrap();
    assert_ne!(first, second);
    assert_eq!(first.parent(), Some(directory.path()));
    let text = fs::read_to_string(first).unwrap();
    assert!(!text.contains("fixture-secret"));
    let document = json::parse(&text).unwrap();
    assert_eq!(document.get("format_version"), Some(&Value::number(1)));
    assert_eq!(
        document.get("messages"),
        Some(&Value::Array(
            archive
                .redactor
                .value(&Value::Array(original.clone()))
                .as_array()
                .unwrap()
                .to_vec()
        ))
    );
    assert_eq!(*archive.messages.lock().unwrap(), original);
    assert!(text.contains("opaque reasoning"));
}

#[test]
fn an_existing_archive_observes_clear_and_export_errors_are_recoverable() {
    let directory = Directory::new();
    let mut agent = Agent::new(
        OpenRouter::fixture("http://127.0.0.1:1/chat/completions".into()),
        Tools::new(directory.path()).unwrap(),
    );
    let archive = agent.archive();
    archive
        .messages
        .lock()
        .unwrap()
        .push(Value::string("previous conversation"));
    agent.clear();
    assert_eq!(
        archive
            .document()
            .get("messages")
            .unwrap()
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert!(
        !fs::read_to_string(archive.save().unwrap())
            .unwrap()
            .contains("previous conversation")
    );
    let mut unavailable = archive.clone();
    unavailable.directory = directory.path().join("missing");
    assert!(unavailable.save().unwrap_err().contains("Could not create"));
    assert!(archive.save().is_ok());
}
