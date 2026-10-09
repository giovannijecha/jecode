use super::*;

#[test]
fn compact_prompt_retains_conditions_and_references_without_repeating_private_versions() {
    let entry = Value::object([
        ("path", Value::string("tests/basics.rs")),
        ("state", Value::string("violated")),
        ("registered_request", Value::number(1)),
        ("history_reference", Value::string("history:3")),
        ("require_check", Value::Bool(true)),
        ("expected_current", Value::string("current-token")),
        ("baseline", Value::string("private baseline metadata")),
        ("current", Value::string("current version metadata")),
        ("snapshot_id", Value::string("private snapshot")),
        (
            "reason",
            Value::string("Original exact constraint. ".repeat(100)),
        ),
    ]);
    let facts = Facts {
        entries: vec![entry.clone()],
    };
    let projected = facts.preview();
    let compact = &projected.get("entries").unwrap().as_array().unwrap()[0];
    for key in [
        "path",
        "state",
        "registered_request",
        "history_reference",
        "require_check",
        "expected_current",
    ] {
        assert_eq!(compact.get(key), entry.get(key));
    }
    for key in ["baseline", "current", "snapshot_id", "reason"] {
        assert!(compact.get(key).is_none());
    }
    assert_eq!(facts.entries[0], entry);
    assert!(facts.problem(true).is_some());
}

#[test]
fn unresolved_entries_take_priority_and_incomplete_registration_stays_actionable() {
    let mut facts = Facts::default();
    for index in 0..10 {
        facts.entries.push(Value::object([
            ("path", Value::string(format!("preserved-{index}"))),
            ("state", Value::string("preserved")),
        ]));
    }
    let marker = Value::object([
        (
            "path",
            Value::string("Incomplete registration at history:7"),
        ),
        ("state", Value::string("unknown")),
        ("registration_incomplete", Value::Bool(true)),
        ("declared_paths", Value::Array(vec![Value::string("tests")])),
        ("registered_request", Value::number(1)),
        ("history_reference", Value::string("history:7")),
    ]);
    facts.entries.push(marker.clone());
    let preview = facts.preview();
    assert_eq!(
        preview.get("active_count").and_then(Value::as_usize),
        Some(11)
    );
    let entries = preview.get("entries").unwrap().as_array().unwrap();
    assert_eq!(entries.len(), 8);
    assert_eq!(entries[0], marker);
}
