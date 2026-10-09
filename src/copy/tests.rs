use super::*;
use crate::{export::Archive, json::Value, redact::Redactor};
use std::sync::{Arc, Mutex};

fn archive(messages: Vec<Value>) -> Archive {
    Archive {
        model: "fixture/model".into(),
        directory: "fixture".into(),
        messages: Arc::new(Mutex::new(messages)),
        events: Arc::new(Mutex::new(vec![])),
        attachments: None,
        redactor: Redactor::new("fixture-private-key".into()),
        effort: "default".into(),
    }
}
fn message(role: &str, text: &str) -> Value {
    Value::object([
        ("role", Value::string(role)),
        ("content", Value::string(text)),
    ])
}

#[test]
fn selects_completed_source_and_excludes_local_tool_and_partial_text() {
    let archive = archive(vec![
        message("assistant", "Old"),
        message("assistant", "Exact\r\n  source  "),
        message("tool", "Tool output"),
        message("user", "next"),
        message("assistant", " "),
    ]);
    archive.events.lock().unwrap().push(Value::object([
        ("type", Value::string("turn_error")),
        ("partial_text", Value::string("Incomplete")),
    ]));
    assert_eq!(response(&archive).unwrap()[0].text, "Exact\r\n  source  ");
}

#[test]
fn no_completed_response_is_an_error_and_credentials_are_masked() {
    assert!(response(&archive(vec![message("user", "hello")])).is_err());
    let targets = response(&archive(vec![message(
        "assistant",
        "```\nfixture-private-key\n```\n",
    )]))
    .unwrap();
    assert_eq!(targets[1].text, "[redacted]\n");
}

#[test]
fn selection_is_a_snapshot_of_the_completed_message() {
    let archive = archive(vec![message("assistant", "First")]);
    let selected = response(&archive).unwrap();
    archive
        .messages
        .lock()
        .unwrap()
        .push(message("assistant", "Second"));
    assert_eq!(selected[0].text, "First");
    assert_eq!(response(&archive).unwrap()[0].text, "Second");
}

#[test]
fn preview_is_bounded_and_control_characters_remain_data() {
    let value = format!("\n\n\x1b{}", "é".repeat(200));
    assert_eq!(preview(&value).chars().count(), 73);
    assert!(preview(&value).starts_with(' '));
    assert!(!preview(&value).contains('\x1b'));
}
