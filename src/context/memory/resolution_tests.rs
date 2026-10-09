use super::*;

fn entry(description: &str, kind: &str, at: usize) -> Value {
    Value::object([
        ("description", Value::string(description)),
        ("kind", Value::string(kind)),
        (
            "evidence",
            Value::Array(vec![Value::string(format!("history:{at}"))]),
        ),
    ])
}

fn history() -> Vec<Value> {
    vec![
        Value::object([
            ("role", Value::string("user")),
            ("content", Value::string("Keep data.txt; build an index.")),
        ]),
        Value::object([
            ("role", Value::string("assistant")),
            ("content", Value::string("Unfinished work")),
        ]),
        Value::object([
            ("role", Value::string("user")),
            (
                "content",
                Value::string("Cancel the index and allow data.txt changes. Audit the report."),
            ),
        ]),
        Value::object([
            ("role", Value::string("assistant")),
            (
                "tool_calls",
                Value::Array(vec![Value::object([
                    ("id", Value::string("verify")),
                    (
                        "function",
                        Value::object([
                            ("name", Value::string("bash")),
                            (
                                "arguments",
                                Value::string(r#"{"command":"verify report","check":true}"#),
                            ),
                        ]),
                    ),
                ])]),
            ),
        ]),
        Value::object([
            ("role", Value::string("tool")),
            ("tool_call_id", Value::string("verify")),
            (
                "content",
                Value::string(
                    Value::object([
                        ("exit_code", Value::number(0)),
                        ("check", Value::Bool(true)),
                        ("check_status", Value::string("passed")),
                    ])
                    .encode(),
                ),
            ),
        ]),
    ]
}

fn state(key: &str, items: &[&str]) -> Value {
    let mut state = json::parse(&fixture("Audit the report")).unwrap();
    if let Value::Object(fields) = &mut state {
        fields.insert(
            key.into(),
            Value::Array(items.iter().map(|text| Value::string(*text)).collect()),
        );
    }
    state
}

#[test]
fn a_new_user_resolution_removes_the_exact_item_even_when_the_model_also_carries_it() {
    for key in ["remaining", "constraints"] {
        let previous = state(key, &["Old index requirement", "Keep unrelated work"]);
        let label = references::label(key, "Old index requirement");
        let mut candidate = state(
            key,
            &[&label, "Keep unrelated work", "New audit requirement"],
        );
        if let Value::Object(fields) = &mut candidate {
            fields.insert(
                "resolved".into(),
                Value::Array(vec![entry(&label, "decision", 2)]),
            );
        }
        let prepared =
            prepare(&candidate.encode(), &previous.encode(), &history(), 2, 8192).unwrap();
        let prepared = json::parse(&prepared).unwrap();
        let items = strings(&prepared, key).unwrap();
        assert!(!items.contains(&"Old index requirement"));
        assert!(items.contains(&"Keep unrelated work"));
        assert!(items.contains(&"New audit requirement"));
        assert_eq!(
            prepared.get("resolved"),
            Some(&Value::Array(vec![entry(
                "Old index requirement",
                "decision",
                2
            )]))
        );
    }
}

#[test]
fn a_new_passed_check_removes_repeated_pending_work_without_losing_its_proof() {
    let mut previous = state("remaining", &["Verify report", "Inspect another file"]);
    let original = entry("Original user scope", "decision", 0);
    if let Value::Object(fields) = &mut previous {
        fields.insert("completed".into(), Value::Array(vec![original.clone()]));
    }
    let label = references::label("remaining", "Verify report");
    let mut candidate = state(
        "remaining",
        &[&label, "Inspect another file", "Deliver the current audit"],
    );
    if let Value::Object(fields) = &mut candidate {
        fields.insert(
            "completed".into(),
            Value::Array(vec![entry(&label, "check", 4)]),
        );
    }
    let prepared = prepare(&candidate.encode(), &previous.encode(), &history(), 2, 8192).unwrap();
    let value = json::parse(&prepared).unwrap();
    let remaining = strings(&value, "remaining").unwrap();
    assert!(!remaining.contains(&"Verify report"));
    assert!(remaining.contains(&"Inspect another file"));
    assert!(remaining.contains(&"Deliver the current audit"));
    assert_eq!(
        value.get("completed"),
        Some(&Value::Array(vec![
            entry("Verify report", "check", 4),
            original
        ]))
    );
}

#[test]
fn stale_or_invalid_proof_cannot_clear_repeated_items() {
    let previous = state("remaining", &["Verify report"]);
    for (kind, at, succeeds) in [
        ("decision", 0, true),
        ("check", 4, true),
        ("decision", 3, false),
    ] {
        let mut candidate = previous.clone();
        if let Value::Object(fields) = &mut candidate {
            fields.insert(
                "resolved".into(),
                Value::Array(vec![entry("Verify report", kind, at)]),
            );
        }
        let result = prepare(&candidate.encode(), &previous.encode(), &history(), 5, 8192);
        assert_eq!(result.is_ok(), succeeds);
        if let Ok(prepared) = result {
            assert!(
                strings(&json::parse(&prepared).unwrap(), "remaining")
                    .unwrap()
                    .contains(&"Verify report")
            );
        }
    }
}

#[test]
fn an_owned_request_boundary_does_not_make_an_old_tool_check_fresh() {
    let mut previous = state("remaining", &["Verify report"]);
    if let Value::Object(fields) = &mut previous {
        fields.insert("reviewed_request_history".into(), Value::number(0));
    }
    let mut candidate = previous.clone();
    if let Value::Object(fields) = &mut candidate {
        fields.insert(
            "resolved".into(),
            Value::Array(vec![entry("Verify report", "check", 4)]),
        );
    }
    let prepared = prepare(&candidate.encode(), &previous.encode(), &history(), 5, 8192).unwrap();
    let ledger = json::parse(&prepared).unwrap();
    assert_eq!(
        strings(&ledger, "remaining").unwrap(),
        vec!["Verify report"]
    );
    assert_eq!(
        ledger
            .get("reviewed_request_history")
            .and_then(Value::as_usize),
        Some(2)
    );
}

#[test]
fn a_passed_check_cannot_withdraw_a_file_constraint() {
    let previous = state("constraints", &["Keep data.txt"]);
    let mut candidate = previous.clone();
    if let Value::Object(fields) = &mut candidate {
        fields.insert(
            "resolved".into(),
            Value::Array(vec![entry("Keep data.txt", "check", 4)]),
        );
    }
    let prepared = prepare(&candidate.encode(), &previous.encode(), &history(), 2, 8192).unwrap();
    assert!(
        strings(&json::parse(&prepared).unwrap(), "constraints")
            .unwrap()
            .contains(&"Keep data.txt")
    );
}

#[test]
fn a_mixed_valid_and_invalid_resolution_is_rejected_atomically() {
    let previous = state("remaining", &["Old index requirement"]);
    let mut candidate = previous.clone();
    let mut resolution = entry("Old index requirement", "decision", 2);
    if let Value::Object(fields) = &mut resolution {
        fields.insert(
            "evidence".into(),
            Value::Array(vec![Value::string("history:2"), Value::string("history:3")]),
        );
    }
    if let Value::Object(fields) = &mut candidate {
        fields.insert("resolved".into(), Value::Array(vec![resolution]));
    }
    let error = prepare(&candidate.encode(), &previous.encode(), &history(), 2, 8192).unwrap_err();
    assert!(error.contains("history:3 is assistant"), "{error}");
    assert!(
        strings(&previous, "remaining")
            .unwrap()
            .contains(&"Old index requirement")
    );
}
