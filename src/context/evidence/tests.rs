use super::*;
use crate::test_support::tool_call;

fn command(check: bool, status: &str, exit: Value) -> Value {
    Value::object([
        ("check", Value::Bool(check)),
        ("check_status", Value::string(status)),
        ("exit_code", exit),
    ])
}

#[test]
fn only_recorded_checks_have_verdicts_and_changes_make_previous_checks_stale() {
    let mut facts = Evidence::default();
    let args = Value::object([("command", Value::string("tests"))]);
    facts.record(
        3,
        "bash",
        &args,
        &command(false, "passed", Value::number(0)),
    );
    assert!(
        facts
            .value()
            .get("checks")
            .unwrap()
            .as_array()
            .unwrap()
            .is_empty()
    );
    facts.record(5, "bash", &args, &command(true, "passed", Value::number(0)));
    facts.record(
        7,
        "write",
        &Value::object([("path", Value::string("src/main.rs"))]),
        &Value::object([("bytes_written", Value::number(42))]),
    );
    facts.record(
        9,
        "bash",
        &args,
        &command(true, "failed", Value::number(101)),
    );
    facts.record(11, "bash", &args, &command(true, "unknown", Value::Null));
    let value = facts.value();
    let checks = value.get("checks").unwrap().as_array().unwrap();
    assert_eq!(
        checks[0].get("after_last_tracked_change"),
        Some(&Value::Bool(false))
    );
    assert_eq!(
        checks[1].get("check_status").and_then(Value::as_str),
        Some("failed")
    );
    assert_eq!(
        checks[2].get("check_status").and_then(Value::as_str),
        Some("unknown")
    );
    assert_eq!(
        value
            .get("last_tracked_change")
            .unwrap()
            .get("message")
            .and_then(Value::as_usize),
        Some(7)
    );
}

#[test]
fn legacy_history_rebuilds_facts_and_an_unexecuted_write_is_not_a_change() {
    let messages = vec![
        Value::object([
            ("role", Value::string("assistant")),
            (
                "tool_calls",
                Value::Array(vec![
                    tool_call(
                        "check",
                        "bash",
                        Value::object([
                            ("command", Value::string("tests")),
                            ("check", Value::Bool(true)),
                        ]),
                    ),
                    tool_call(
                        "write",
                        "write",
                        Value::object([("path", Value::string("src/main.rs"))]),
                    ),
                ]),
            ),
        ]),
        Value::object([
            ("role", Value::string("tool")),
            ("tool_call_id", Value::string("check")),
            (
                "content",
                Value::string(command(true, "passed", Value::number(0)).encode()),
            ),
        ]),
        Value::object([
            ("role", Value::string("tool")),
            ("tool_call_id", Value::string("write")),
            (
                "content",
                Value::string(Value::object([("error", Value::string("Not executed"))]).encode()),
            ),
        ]),
    ];
    let mut facts = Evidence::default();
    facts.rebuild(&messages);
    facts.validate(&messages).unwrap();
    let encoded = facts.value();
    assert_eq!(encoded.get("last_tracked_change"), Some(&Value::Null));
    assert!(encoded.encode().contains("passed"));
    let restored = Evidence::parse(Some(&encoded)).unwrap();
    assert_eq!(restored.value(), encoded);
}

fn file_change(source: &str, restored: bool) -> Value {
    Value::object([
        ("path", Value::string("tests/protected.rs")),
        ("source", Value::string(source)),
        ("restored_to_first_observed", Value::Bool(restored)),
    ])
}

fn changed_command(source: &str, restored: bool) -> Value {
    let mut result = command(true, "passed", Value::number(0));
    if let Value::Object(fields) = &mut result {
        fields.insert(
            "file_changes".into(),
            Value::Array(vec![file_change(source, restored)]),
        );
    }
    result
}

#[test]
fn a_mutating_check_cannot_revalidate_earlier_checks_and_restoration_requires_a_fresh_check() {
    let mut facts = Evidence::default();
    let args = Value::object([("command", Value::string("cargo fmt"))]);
    facts.record(3, "bash", &args, &command(true, "passed", Value::number(0)));
    facts.record(5, "bash", &args, &changed_command("during_command", false));
    let value = facts.value();
    let checks = value.get("checks").unwrap().as_array().unwrap();
    assert!(
        checks
            .iter()
            .all(|check| check.get("after_last_tracked_change") == Some(&Value::Bool(false)))
    );
    assert!(facts.completion_notice().is_some());
    let restored = Evidence::parse(Some(&value)).unwrap();
    assert_eq!(restored.value(), value);
    facts.record(7, "bash", &args, &changed_command("during_command", true));
    assert!(
        facts
            .value()
            .get("observed_indirect_changes")
            .unwrap()
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert!(facts.completion_notice().is_some());
    facts.record(9, "bash", &args, &command(true, "passed", Value::number(0)));
    assert!(facts.completion_notice().is_none());
}

#[test]
fn changes_before_a_check_are_in_its_input_but_inspection_does_not_prove_intended_scope() {
    let mut facts = Evidence::default();
    let args = Value::object([("command", Value::string("cargo test"))]);
    facts.record(3, "bash", &args, &changed_command("before_command", false));
    assert_eq!(
        facts.value().get("checks").unwrap().as_array().unwrap()[0]
            .get("after_last_tracked_change"),
        Some(&Value::Bool(true))
    );
    assert!(facts.completion_notice().is_some());
    facts.record(
        5,
        "read",
        &Value::object([("path", Value::string("tests/protected.rs"))]),
        &Value::object([(
            "file_observations",
            Value::Array(vec![Value::object([(
                "path",
                Value::string("tests/protected.rs"),
            )])]),
        )]),
    );
    assert!(facts.completion_notice().is_none());
    assert!(
        facts.needs_attention(),
        "Inspection must not silently approve the change"
    );
    facts.finish_review();
    assert!(!facts.needs_attention());
    assert_eq!(
        facts
            .value()
            .get("observed_indirect_changes")
            .unwrap()
            .as_array()
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn incomplete_comparisons_remain_visible_even_without_a_recorded_check() {
    let mut facts = Evidence::default();
    facts.record(
        3,
        "bash",
        &Value::object([("command", Value::string("true"))]),
        &Value::object([
            ("exit_code", Value::number(0)),
            (
                "file_tracking",
                Value::object([
                    ("status", Value::string("incomplete")),
                    (
                        "errors",
                        Value::Array(vec![Value::object([("path", Value::string("protected"))])]),
                    ),
                ]),
            ),
        ]),
    );
    assert!(facts.needs_attention());
    assert_eq!(
        facts.prompt_value().get("uninspected_file_change_count"),
        Some(&Value::number(1))
    );
}

#[test]
fn acknowledged_indirect_changes_remain_recorded_without_reopening_review_on_resume() {
    let messages = vec![
        Value::object([
            ("role", Value::string("assistant")),
            (
                "tool_calls",
                Value::Array(vec![tool_call(
                    "check",
                    "bash",
                    Value::object([
                        ("command", Value::string("tests")),
                        ("check", Value::Bool(true)),
                    ]),
                )]),
            ),
        ]),
        Value::object([
            ("role", Value::string("tool")),
            ("tool_call_id", Value::string("check")),
            (
                "content",
                Value::string(changed_command("before_command", false).encode()),
            ),
        ]),
        Value::object([
            ("role", Value::string("assistant")),
            (
                "tool_calls",
                Value::Array(vec![tool_call(
                    "read",
                    "read",
                    Value::object([("path", Value::string("tests/protected.rs"))]),
                )]),
            ),
        ]),
        Value::object([
            ("role", Value::string("tool")),
            ("tool_call_id", Value::string("read")),
            (
                "content",
                Value::string(
                    Value::object([(
                        "file_observations",
                        Value::Array(vec![Value::object([(
                            "path",
                            Value::string("tests/protected.rs"),
                        )])]),
                    )])
                    .encode(),
                ),
            ),
        ]),
    ];
    let mut facts = Evidence::default();
    facts.rebuild(&messages);
    assert!(facts.needs_attention());
    facts.finish_review();
    assert!(!facts.needs_attention());
    let mut resumed = Evidence::parse(Some(&facts.value())).unwrap();
    resumed.rebuild(&messages);
    resumed.validate(&messages).unwrap();
    assert!(!resumed.needs_attention());
    assert_eq!(
        resumed
            .value()
            .get("observed_indirect_changes")
            .unwrap()
            .as_array()
            .unwrap()
            .len(),
        1
    );
}
