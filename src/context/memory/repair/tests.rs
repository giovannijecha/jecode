use super::*;
use crate::test_support::tool_call;

fn messages(check: bool, result: Value) -> Vec<Value> {
    vec![
        Value::object([
            ("role", Value::string("user")),
            (
                "content",
                Value::string("Keep caffè ☕ exactly; verify cargo test."),
            ),
        ]),
        Value::object([
            ("role", Value::string("assistant")),
            (
                "tool_calls",
                Value::Array(vec![tool_call(
                    "run",
                    "bash",
                    Value::object([
                        ("command", Value::string("cargo test --locked --offline")),
                        ("check", Value::Bool(check)),
                    ]),
                )]),
            ),
        ]),
        Value::object([
            ("role", Value::string("tool")),
            ("tool_call_id", Value::string("run")),
            ("content", Value::string(result.encode())),
        ]),
    ]
}

fn facts(messages: &[Value]) -> Vec<Value> {
    let text = sources(messages, 0..messages.len());
    json::parse(text.split_once(":\n").unwrap().1)
        .unwrap()
        .as_array()
        .unwrap()
        .to_vec()
}

#[test]
fn repair_exposes_exact_requests_targets_and_native_check_eligibility() {
    let result = Value::object([
        ("exit_code", Value::number(0)),
        ("check", Value::Bool(true)),
        ("check_status", Value::string("passed")),
        (
            "stdout",
            Value::string("Large compiler output ".repeat(1000)),
        ),
    ]);
    for check in [false, true] {
        let original = messages(check, result.clone());
        let before = original.clone();
        let view = facts(&original);
        assert_eq!(view[0].get("content"), original[0].get("content"));
        assert_eq!(
            view[2].get("eligible_proof"),
            crate::context::memory::source_record(&original, 2).get("eligible_proof")
        );
        let kinds = view[2].get("eligible_proof").unwrap().as_array().unwrap();
        assert_eq!(kinds.contains(&Value::string("check")), check);
        assert_eq!(
            view[2].get("call_arguments").unwrap().get("command"),
            Some(&Value::string("cargo test --locked --offline"))
        );
        assert_eq!(
            view[2].get("result_facts").unwrap().get("check_status"),
            Some(&Value::string("passed"))
        );
        assert!(!sources(&original, 0..3).contains("Large compiler output"));
        assert_eq!(original, before);
    }
}

#[test]
fn failure_and_uncertainty_are_preserved_without_new_completion_proof() {
    for marker in [
        ("exit_code", Value::number(1)),
        ("cancelled", Value::Bool(true)),
        ("timed_out", Value::Bool(true)),
        ("outcome", Value::string("unknown")),
        ("error", Value::string("Native execution failed")),
    ] {
        let result = Value::object([marker.clone()]);
        let original = messages(true, result);
        let view = facts(&original);
        assert_eq!(
            view[2].get("result_facts").unwrap().get(marker.0),
            Some(&marker.1)
        );
        let kinds = view[2].get("eligible_proof").unwrap().as_array().unwrap();
        assert!(!kinds.contains(&Value::string("change")));
        assert!(!kinds.contains(&Value::string("check")));
    }
}

#[test]
fn matching_call_provenance_survives_an_intervening_user_request() {
    let mut original = messages(true, Value::object([("exit_code", Value::number(0))]));
    original.insert(
        2,
        Value::object([
            ("role", Value::string("user")),
            ("content", Value::string("New request")),
        ]),
    );
    let view = facts(&original);
    assert_eq!(
        view[3].get("request_history"),
        Some(&Value::string("history:0"))
    );
    assert_eq!(
        view[3].get("call_history"),
        Some(&Value::string("history:1"))
    );
    assert!(view[3].get("call_arguments").is_some());
}

#[test]
fn missing_or_malformed_sources_supply_no_invented_native_facts() {
    let original = vec![Value::object([
        ("role", Value::string("tool")),
        ("tool_call_id", Value::string("absent")),
        ("content", Value::string("{incomplete")),
    ])];
    let text = sources(&original, [0, 9]);
    let view = json::parse(text.split_once(":\n").unwrap().1).unwrap();
    let entries = view.as_array().unwrap();
    assert!(entries[0].get("result_facts").is_none());
    assert!(entries[0].get("call_arguments").is_none());
    assert_eq!(entries[1].get("role"), Some(&Value::string("missing")));
    assert_eq!(
        entries[1].get("eligible_proof"),
        Some(&Value::Array(vec![]))
    );
}

#[test]
fn incomplete_or_over_budget_updates_keep_the_original_transcript_repair() {
    let candidate = crate::context::memory::fixture("Review source");
    assert!(has_update_facts(&candidate, 4096));
    for text in ["Done", "{}", r#"{"objective":"Only an objective"}"#] {
        assert!(!has_update_facts(text, 4096));
    }
    assert!(!has_update_facts(&candidate, 16));
}
