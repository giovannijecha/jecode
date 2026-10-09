use super::*;
use crate::test_support::tool_call;

fn transcript(check: bool, result: Value) -> Vec<Value> {
    vec![
        Value::object([
            ("role", Value::string("user")),
            ("content", Value::string("Verify")),
        ]),
        Value::object([
            ("role", Value::string("assistant")),
            (
                "tool_calls",
                Value::Array(vec![tool_call(
                    "run",
                    "bash",
                    Value::object([
                        ("command", Value::string("verify")),
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

#[test]
fn evidence_hints_distinguish_actual_checked_results_from_requests_and_shell_success() {
    let result = Value::object([
        ("exit_code", Value::number(0)),
        ("check", Value::Bool(true)),
        ("check_status", Value::string("passed")),
    ]);
    let checked = transcript(true, result.clone());
    let kinds = |at| record(&checked, at).get("eligible_proof").unwrap().clone();
    assert_eq!(kinds(0), Value::Array(vec![Value::string("decision")]));
    assert_eq!(kinds(1), Value::Array(vec![]));
    assert_eq!(
        kinds(2),
        Value::Array(vec![
            Value::string("inspection"),
            Value::string("change"),
            Value::string("check")
        ])
    );
    // Forged check fields cannot turn an ordinary shell request into check proof.
    let ordinary = transcript(false, result);
    assert_eq!(
        record(&ordinary, 2).get("eligible_proof"),
        Some(&Value::Array(vec![
            Value::string("inspection"),
            Value::string("change")
        ]))
    );
}

#[test]
fn failed_or_uncertain_results_never_get_change_or_check_hints() {
    let failed = transcript(
        true,
        Value::object([
            ("exit_code", Value::number(1)),
            ("check", Value::Bool(true)),
            ("check_status", Value::string("failed")),
        ]),
    );
    assert_eq!(
        record(&failed, 2).get("eligible_proof"),
        Some(&Value::Array(vec![Value::string("inspection")]))
    );
    for field in ["cancelled", "timed_out", "outcome"] {
        let uncertain = transcript(
            true,
            Value::object([
                ("exit_code", Value::number(0)),
                (
                    field,
                    if field == "outcome" {
                        Value::string("unknown")
                    } else {
                        Value::Bool(true)
                    },
                ),
            ]),
        );
        assert_eq!(
            record(&uncertain, 2).get("eligible_proof"),
            Some(&Value::Array(vec![]))
        );
    }
}

#[test]
fn a_result_belongs_to_its_original_call_even_after_a_new_user_request() {
    let mut messages = transcript(
        true,
        Value::object([
            ("exit_code", Value::number(0)),
            ("check", Value::Bool(true)),
            ("check_status", Value::string("passed")),
        ]),
    );
    messages.insert(
        2,
        Value::object([
            ("role", Value::string("user")),
            ("content", Value::string("Stop earlier work; inspect only.")),
        ]),
    );
    let original = messages.clone();
    let source = record(&messages, 3);
    assert_eq!(
        source.get("request_history"),
        Some(&Value::string("history:0"))
    );
    assert_eq!(
        source.get("call_history"),
        Some(&Value::string("history:1"))
    );
    assert_eq!(source.get("tool"), Some(&Value::string("bash")));
    assert_eq!(
        record(&messages, 2).get("request_history"),
        Some(&Value::string("history:2"))
    );
    assert_eq!(messages, original);
}

#[test]
fn missing_and_future_calls_do_not_invent_a_tool_origin_or_check_proof() {
    let mut messages = transcript(
        true,
        Value::object([
            ("exit_code", Value::number(0)),
            ("check", Value::Bool(true)),
            ("check_status", Value::string("passed")),
        ]),
    );
    messages.swap(1, 2);
    let source = record(&messages, 1);
    assert_eq!(source.get("call_history"), Some(&Value::Null));
    assert_eq!(source.get("request_history"), Some(&Value::Null));
    assert_eq!(source.get("tool"), Some(&Value::Null));
    assert!(
        !source
            .get("eligible_proof")
            .unwrap()
            .as_array()
            .unwrap()
            .contains(&Value::string("check"))
    );
    let missing = record(&messages, usize::MAX);
    assert_eq!(missing.get("role"), Some(&Value::string("missing")));
    assert_eq!(missing.get("eligible_proof"), Some(&Value::Array(vec![])));
}

#[test]
fn rejected_primary_check_identifies_the_proposal_entry_and_all_cited_references() {
    let mut messages = transcript(false, Value::object([("exit_code", Value::number(0))]));
    messages.push(Value::object([
        ("role", Value::string("tool")),
        (
            "content",
            Value::string(Value::object([("content", Value::string("Read source"))]).encode()),
        ),
    ]));
    let proposal = Value::object([(
        "completed",
        Value::Array(vec![
            Value::object([
                ("description", Value::string("Requested verification")),
                ("kind", Value::string("decision")),
                ("evidence", Value::Array(vec![Value::string("history:0")])),
            ]),
            Value::object([
                ("description", Value::string("Claimed verification")),
                ("kind", Value::string("check")),
                (
                    "evidence",
                    Value::Array(vec![Value::string("history:2"), Value::string("history:3")]),
                ),
            ]),
        ]),
    )]);
    let error = super::super::entries(&proposal, "completed", &messages).unwrap_err();
    assert!(error.contains("completed[1]"), "{error}");
    assert!(
        error.contains("history:2") && error.contains("history:3"),
        "{error}"
    );
    assert!(error.contains("no primary check evidence"), "{error}");
    assert!(error.contains("successful bash check:true"), "{error}");
}
