use super::*;

fn message(role: &str, content: &str) -> Value {
    Value::object([
        ("role", Value::string(role)),
        ("content", Value::string(content)),
    ])
}

fn history() -> Vec<Value> {
    vec![
        message("system", "Agent"),
        message("user", "Build the index; preserve input.tsv."),
        message("assistant", "Draft created."),
        message("user", "Cancel the index and audit the existing report."),
        message("assistant", "Inspecting the report."),
    ]
}

#[test]
fn archived_requests_cannot_misidentify_a_newer_live_request_as_the_previous_task() {
    let messages = history();
    let original = messages.clone();
    let context = Context {
        from: 3,
        summary: "Index cancelled; audit pending.".into(),
        ..Context::default()
    };
    let projected = context.project(&messages, 0);
    let archive = projected[1].get("content").and_then(Value::as_str).unwrap();
    assert!(
        archive.contains("Last user request in this checkpoint: history:1"),
        "{archive}"
    );
    assert!(archive.contains("history:1 — archived user request:"));
    assert!(!archive.contains("takes precedence"));
    assert_eq!(projected[3], original[3]);
    assert_eq!(messages, original);
}

#[test]
fn a_fully_compacted_request_keeps_its_actual_identity_and_original_instruction() {
    let messages = history();
    let context = Context {
        from: messages.len(),
        summary: "Audit pending.".into(),
        ..Context::default()
    };
    let requests = context.user_requests(&messages);
    assert!(requests.contains("Last user request in this checkpoint: history:3"));
    assert!(requests.contains("history:3 — archived user request:"));
    assert!(requests.contains("Cancel the index and audit the existing report."));
    assert!(requests.contains("Build the index; preserve input.tsv."));
}

#[test]
fn new_user_messages_leave_the_entire_compacted_prefix_unchanged() {
    let mut messages = history();
    let context = Context {
        from: messages.len(),
        summary: "Audit pending; the index was cancelled.".into(),
        skill_references: "Skills read: skill:audit".into(),
        ..Context::default()
    };
    let checkpoint = context.project(&messages, 0);
    messages.push(message("user", "Continue the audit and check totals."));
    messages.push(message("assistant", "Totals checked."));
    messages.push(message("user", "Now inspect the dates."));
    let next = context.project(&messages, 0);
    assert!(next.starts_with(&checkpoint));
    assert_eq!(&next[checkpoint.len()..], &messages[context.from..]);
}

#[test]
fn saved_skill_references_are_optional_for_older_sessions_and_survive_roundtrips() {
    let mut context = Context::default();
    let mut old = context.value();
    if let Value::Object(fields) = &mut old {
        fields.remove("skill_references");
    }
    assert!(
        Context::parse(Some(&old))
            .unwrap()
            .skill_references
            .is_empty()
    );
    context.skill_references = "skill:audit".into();
    assert_eq!(
        Context::parse(Some(&context.value()))
            .unwrap()
            .skill_references,
        "skill:audit"
    );
    if let Value::Object(fields) = &mut old {
        fields.insert("skill_references".into(), Value::Bool(true));
    }
    assert!(Context::parse(Some(&old)).is_err());
}
