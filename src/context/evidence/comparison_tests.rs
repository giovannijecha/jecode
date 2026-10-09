use super::Evidence;
use crate::json::Value;

fn protection(reference: &str, state: &str) -> Value {
    Value::object([
        ("path", Value::string("owner.txt")),
        ("state", Value::string(state)),
        ("history_reference", Value::string(reference)),
        ("snapshot_id", Value::string("native-baseline")),
    ])
}

fn status(reference: &str, state: &str) -> Value {
    Value::object([
        ("file_protections_scope", Value::string("all")),
        (
            "file_protections",
            Value::Array(vec![protection(reference, state)]),
        ),
    ])
}

fn interrupted(paths: &[&str]) -> Value {
    Value::object([
        ("cancelled", Value::Bool(true)),
        (
            "file_tracking",
            Value::object([
                ("status", Value::string("incomplete")),
                (
                    "errors",
                    Value::Array(
                        paths
                            .iter()
                            .map(|path| {
                                Value::object([
                                    ("path", Value::string(*path)),
                                    ("error", Value::string("Comparison was cancelled")),
                                ])
                            })
                            .collect(),
                    ),
                ),
            ]),
        ),
    ])
}

fn pending(facts: &Evidence) -> Vec<Value> {
    facts
        .value()
        .get("observed_indirect_changes")
        .unwrap()
        .as_array()
        .unwrap()
        .to_vec()
}

fn compare(facts: &mut Evidence, result: &Value) {
    facts.record(
        7,
        "protect",
        &Value::object([("action", Value::string("status"))]),
        result,
    );
}

#[test]
fn a_later_native_preservation_comparison_resolves_only_its_earlier_gap() {
    let mut facts = Evidence::default();
    facts.record(
        5,
        "bash",
        &Value::Null,
        &interrupted(&["owner.txt", "editable.rs"]),
    );
    compare(&mut facts, &status("history:3", "preserved"));
    let remaining = pending(&facts);
    assert_eq!(remaining.len(), 1);
    assert_eq!(
        remaining[0].get("path").and_then(Value::as_str),
        Some("editable.rs")
    );
    assert!(facts.completion_notice().unwrap().starts_with("1 observed"));
}

#[test]
fn known_changes_still_require_inspection_even_if_bytes_match_a_protection() {
    let mut facts = Evidence::default();
    facts.record(
        5,
        "bash",
        &Value::Null,
        &Value::object([(
            "file_changes",
            Value::Array(vec![Value::object([
                ("path", Value::string("owner.txt")),
                ("source", Value::string("during_command")),
            ])]),
        )]),
    );
    compare(&mut facts, &status("history:3", "preserved"));
    assert_eq!(pending(&facts).len(), 1);
    assert!(facts.needs_attention());
}

#[test]
fn incomplete_or_newer_baselines_cannot_resolve_an_earlier_gap() {
    for result in [
        status("history:6", "preserved"),
        status("history:5", "preserved"),
        status("history:3", "unknown"),
        status("history:3", "violated"),
        status("history:3", "released"),
        status("history:requests", "preserved"),
        Value::object([
            ("file_protections_scope", Value::string("all")),
            (
                "file_protections",
                Value::Array(vec![Value::object([
                    ("path", Value::string("owner.txt")),
                    ("state", Value::string("preserved")),
                ])]),
            ),
        ]),
    ] {
        let mut facts = Evidence::default();
        facts.record(5, "bash", &Value::Null, &interrupted(&["owner.txt"]));
        compare(&mut facts, &result);
        assert_eq!(pending(&facts).len(), 1, "{}", result.encode());
    }
}

#[test]
fn failed_or_unstarted_status_does_not_promote_cached_preservation() {
    for marker in [
        ("error", Value::string("Status was not executed")),
        ("outcome", Value::string("not_started")),
        ("outcome", Value::string("unknown")),
        ("cancelled", Value::Bool(true)),
    ] {
        let mut facts = Evidence::default();
        facts.record(5, "bash", &Value::Null, &interrupted(&["owner.txt"]));
        let mut result = status("history:3", "preserved");
        if let Value::Object(fields) = &mut result {
            fields.insert(marker.0.into(), marker.1);
        }
        compare(&mut facts, &result);
        assert_eq!(pending(&facts).len(), 1);
    }
}

#[test]
fn preservation_cannot_replace_a_failed_or_stale_check() {
    let mut facts = Evidence::default();
    let mut failed = interrupted(&["owner.txt"]);
    if let Value::Object(fields) = &mut failed {
        fields.insert("check".into(), Value::Bool(true));
        fields.insert("check_status".into(), Value::string("cancelled"));
    }
    facts.record(5, "bash", &Value::Null, &failed);
    compare(&mut facts, &status("history:3", "preserved"));
    assert!(pending(&facts).is_empty());
    assert!(facts.needs_attention());
    assert!(
        facts
            .completion_notice()
            .unwrap()
            .contains("latest recorded check did not pass")
    );
    let mut stale = Evidence::default();
    stale.record(
        3,
        "bash",
        &Value::Null,
        &Value::object([
            ("exit_code", Value::number(0)),
            ("check", Value::Bool(true)),
            ("check_status", Value::string("passed")),
        ]),
    );
    stale.record(5, "bash", &Value::Null, &interrupted(&["owner.txt"]));
    compare(&mut stale, &status("history:3", "preserved"));
    assert!(pending(&stale).is_empty());
    assert!(stale.needs_attention());
    assert!(stale.completion_notice().unwrap().contains("predates"));
}

fn pair(name: &str, arguments: Value, result: Value, id: &str) -> [Value; 2] {
    [
        Value::object([
            ("role", Value::string("assistant")),
            (
                "tool_calls",
                Value::Array(vec![Value::object([
                    ("id", Value::string(id)),
                    (
                        "function",
                        Value::object([
                            ("name", Value::string(name)),
                            ("arguments", Value::string(arguments.encode())),
                        ]),
                    ),
                ])]),
            ),
        ]),
        Value::object([
            ("role", Value::string("tool")),
            ("tool_call_id", Value::string(id)),
            ("content", Value::string(result.encode())),
        ]),
    ]
}

#[test]
fn replay_and_saved_evidence_reconcile_without_rewriting_original_history() {
    let mut messages = vec![Value::object([("role", Value::string("system"))])];
    messages.extend(pair(
        "protect",
        Value::object([("action", Value::string("record"))]),
        status("history:2", "preserved"),
        "registration",
    ));
    messages.extend(pair(
        "bash",
        Value::Null,
        interrupted(&["owner.txt"]),
        "interruption",
    ));
    messages.extend(pair(
        "protect",
        Value::object([("action", Value::string("status"))]),
        status("history:2", "preserved"),
        "comparison",
    ));
    let original = messages.clone();
    let mut facts = Evidence::default();
    facts.rebuild(&messages);
    assert!(pending(&facts).is_empty());
    assert!(!facts.needs_attention());
    let saved = facts.value();
    assert_eq!(Evidence::parse(Some(&saved)).unwrap().value(), saved);
    assert_eq!(messages, original);
}
