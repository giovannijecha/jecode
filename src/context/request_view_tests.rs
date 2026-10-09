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
    let archive = projected[2].get("content").and_then(Value::as_str).unwrap();
    assert!(
        archive
            .contains("Latest original user request: history:3 (retained in the live transcript)"),
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
    assert!(requests.contains("Latest original user request: history:3 (included below)"));
    assert!(requests.contains("history:3 — current original user request:"));
    assert!(requests.contains("Cancel the index and audit the existing report."));
    assert!(requests.contains("Build the index; preserve input.tsv."));
}
