use super::*;
use crate::attachments::{Attachment, Prompt};

fn message(role: &str, content: &str) -> Value {
    Value::object([
        ("role", Value::string(role)),
        ("content", Value::string(content)),
    ])
}

fn returned(from: usize, after: usize) -> Value {
    Value::object([
        ("type", Value::string("turn_returned")),
        ("from_message", Value::number(from)),
        ("after_message", Value::number(after)),
    ])
}

fn history() -> Vec<Value> {
    let image = Attachment {
        id: "att-1-2-3".into(),
        name: "failed.png".into(),
        media: "image/png".into(),
        size: 1024,
        width: Some(16),
        height: Some(16),
        pages: None,
    };
    vec![
        message("system", "Agent"),
        message("user", "Original task"),
        Prompt::new("Failed prompt".into(), vec![image]).message(),
        message("user", "Additional unanswered input"),
        message("user", "Continue without the image"),
    ]
}

#[test]
fn returned_turn_is_absent_from_live_projection_including_its_attachment() {
    let messages = history();
    let original = messages.clone();
    let mut context = Context::default();
    context.restore_returned(&[returned(2, 4)], messages.len());

    let projected = context.project(&messages, 0);
    assert_eq!(
        projected,
        vec![
            messages[0].clone(),
            messages[1].clone(),
            messages[4].clone()
        ]
    );
    let encoded = projected.iter().map(Value::encode).collect::<String>();
    assert!(!encoded.contains("Failed prompt"));
    assert!(!encoded.contains("att-1-2-3"));
    assert!(!encoded.contains("Additional unanswered input"));
    assert_eq!(messages, original);
}

#[test]
fn returned_prompt_is_absent_from_archived_user_requests() {
    let messages = history();
    let mut context = Context {
        from: messages.len(),
        summary: "Continue the original task.".into(),
        ..Context::default()
    };
    context.restore_returned(&[returned(2, 4)], messages.len());

    let requests = context.user_requests(&messages);
    assert!(requests.contains("Original task"));
    assert!(requests.contains("Continue without the image"));
    assert!(requests.contains("Last user request in this checkpoint: history:4"));
    assert!(!requests.contains("Failed prompt"));
    assert!(!requests.contains("att-1-2-3"));

    let projected = context.project(&messages, 0);
    let archive = projected[1].get("content").and_then(Value::as_str).unwrap();
    assert!(!archive.contains("Failed prompt"));
    assert!(!archive.contains("att-1-2-3"));
}

#[test]
fn restoring_ranges_clamps_to_history_without_serializing_a_duplicate_cache() {
    let messages = history();
    let mut context = Context::default();
    context.restore_returned(
        &[
            returned(2, usize::MAX),
            returned(usize::MAX, usize::MAX),
            returned(4, 2),
        ],
        messages.len(),
    );

    assert_eq!(
        context.returned.iter().copied().collect::<Vec<_>>(),
        vec![2, 3, 4]
    );
    assert_eq!(context.input_tokens, None);
    assert_eq!(context.calibration, None);
    assert!(context.value().get("returned").is_none());
    assert!(
        Context::parse(Some(&context.value()))
            .unwrap()
            .returned
            .is_empty()
    );
}
