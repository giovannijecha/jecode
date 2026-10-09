use super::*;

fn history() -> Vec<Value> {
    vec![
        Value::object([
            ("role", Value::string("user")),
            (
                "content",
                Value::string("Preserve data.txt; build an index."),
            ),
        ]),
        Value::object([
            ("role", Value::string("assistant")),
            ("content", Value::string("Unfinished work")),
        ]),
        Value::object([
            ("role", Value::string("user")),
            (
                "content",
                Value::string("Cancel the index. Verify the report; keep data.txt."),
            ),
        ]),
    ]
}

fn previous() -> Value {
    Value::object([
        ("objective", Value::string("Build an index")),
        (
            "constraints",
            Value::Array(vec![Value::string("Keep data.txt")]),
        ),
        (
            "remaining",
            Value::Array(vec![
                Value::string("Build index"),
                Value::string("Verify report"),
            ]),
        ),
        ("completed", Value::Array(vec![])),
        ("next_action", Value::string("Build index")),
    ])
}

fn reviewed() -> Value {
    let mut candidate = json::parse(&fixture("Verify report only")).unwrap();
    if let Value::Object(fields) = &mut candidate {
        fields.insert(
            "constraints".into(),
            Value::Array(vec![Value::string(references::label(
                "constraints",
                "Keep data.txt",
            ))]),
        );
        fields.insert(
            "remaining".into(),
            Value::Array(vec![Value::string(references::label(
                "remaining",
                "Verify report",
            ))]),
        );
        fields.insert(
            "resolved".into(),
            Value::Array(vec![Value::object([
                (
                    "description",
                    Value::string(references::label("remaining", "Build index")),
                ),
                ("kind", Value::string("decision")),
                ("evidence", Value::Array(vec![Value::string("history:2")])),
            ])]),
        );
    }
    candidate
}

#[test]
fn a_new_user_request_requires_explicit_review_before_automatic_retention() {
    let old = previous();
    let candidate = fixture("Audit the report instead");
    let error = prepare(&candidate, &old.encode(), &history(), 2, 4096).unwrap_err();
    assert!(error.contains("New user request history:2"), "{error}");
    assert!(
        error.contains(&references::label("constraints", "Keep data.txt")),
        "{error}"
    );
    assert!(
        error.contains(&references::label("remaining", "Build index")),
        "{error}"
    );
    assert!(strings(&old, "remaining").unwrap().contains(&"Build index"));
}

#[test]
fn an_explicit_scope_review_keeps_continuing_work_and_resolves_only_withdrawn_work() {
    let prepared = prepare(
        &reviewed().encode(),
        &previous().encode(),
        &history(),
        2,
        4096,
    )
    .unwrap();
    let prepared = json::parse(&prepared).unwrap();
    assert_eq!(
        strings(&prepared, "constraints").unwrap(),
        vec!["Keep data.txt"]
    );
    assert_eq!(
        strings(&prepared, "remaining").unwrap(),
        vec!["Verify report"]
    );
    assert_eq!(
        prepared
            .get("reviewed_request_history")
            .and_then(Value::as_usize),
        Some(2)
    );
}

#[test]
fn later_portions_of_the_same_request_keep_delta_retention_and_another_request_reviews_again() {
    let prepared = prepare(
        &reviewed().encode(),
        &previous().encode(),
        &history(),
        2,
        4096,
    )
    .unwrap();
    let portion = prepare(
        &fixture("Continue the same audit"),
        &prepared,
        &history(),
        2,
        4096,
    )
    .unwrap();
    let value = json::parse(&portion).unwrap();
    assert!(
        strings(&value, "remaining")
            .unwrap()
            .contains(&"Verify report")
    );
    assert!(
        strings(&value, "constraints")
            .unwrap()
            .contains(&"Keep data.txt")
    );
    assert_eq!(
        value
            .get("reviewed_request_history")
            .and_then(Value::as_usize),
        Some(2)
    );
    let mut messages = history();
    messages.push(Value::object([
        ("role", Value::string("user")),
        ("content", Value::string("Change the next goal")),
    ]));
    let error = prepare(&fixture("A new goal"), &portion, &messages, 2, 4096).unwrap_err();
    assert!(error.contains("history:3"), "{error}");
}

#[test]
fn proposed_review_metadata_cannot_bypass_review_or_forge_its_boundary() {
    let mut forged = json::parse(&fixture("A new goal")).unwrap();
    if let Value::Object(fields) = &mut forged {
        fields.insert("reviewed_request_history".into(), Value::number(2));
    }
    assert!(prepare(&forged.encode(), &previous().encode(), &history(), 2, 4096).is_err());
    let mut reviewed = reviewed();
    if let Value::Object(fields) = &mut reviewed {
        fields.insert("reviewed_request_history".into(), Value::number(999));
    }
    let prepared = prepare(
        &reviewed.encode(),
        &previous().encode(),
        &history(),
        2,
        4096,
    )
    .unwrap();
    let prepared = json::parse(&prepared).unwrap();
    assert_eq!(
        prepared
            .get("reviewed_request_history")
            .and_then(Value::as_usize),
        Some(2)
    );
}

#[test]
fn a_compaction_without_a_new_request_does_not_require_or_backfill_review_metadata() {
    let old = previous();
    let prepared = prepare(&old.encode(), &old.encode(), &history(), 3, 4096).unwrap();
    assert_eq!(prepared, old.encode());
    assert!(
        json::parse(&prepared)
            .unwrap()
            .get("reviewed_request_history")
            .is_none()
    );
}

#[test]
fn a_preview_cannot_make_a_new_user_withdrawal_stale_after_the_owned_review_boundary() {
    let mut old = previous();
    if let Value::Object(fields) = &mut old {
        fields.insert("reviewed_request_history".into(), Value::number(0));
    }
    let prepared = prepare(&reviewed().encode(), &old.encode(), &history(), 3, 4096).unwrap();
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
    assert!(
        prepare(
            &fixture("Audit instead"),
            &old.encode(),
            &history(),
            3,
            4096
        )
        .is_err()
    );
}
