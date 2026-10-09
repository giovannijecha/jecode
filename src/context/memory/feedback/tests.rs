use super::*;
use crate::test_support::tool_call;

#[test]
fn rejected_assistant_citations_expose_actual_sources_without_changing_the_proposal() {
    let messages = vec![
        Value::object([("role", Value::string("user"))]),
        Value::object([
            ("role", Value::string("assistant")),
            (
                "tool_calls",
                Value::Array(vec![tool_call(
                    "write",
                    "write",
                    Value::object([("path", Value::string("draft.txt"))]),
                )]),
            ),
        ]),
        Value::object([
            ("role", Value::string("tool")),
            ("tool_call_id", Value::string("write")),
            (
                "content",
                Value::string(Value::object([("bytes_written", Value::number(4))]).encode()),
            ),
        ]),
    ];
    let mut proposal = json::parse(&super::super::fixture("Review draft")).unwrap();
    if let Value::Object(fields) = &mut proposal {
        fields.insert(
            "completed".into(),
            Value::Array(vec![Value::object([
                ("description", Value::string("Draft written")),
                ("kind", Value::string("change")),
                (
                    "evidence",
                    Value::Array(vec![Value::string("history:1"), Value::string("history:2")]),
                ),
            ])]),
        );
    }
    let candidate = proposal.encode();
    let error = super::super::prepare(&candidate, "", &messages, 0, 4096).unwrap_err();
    let text = validation_feedback(&candidate, &error, "", &messages, 0, 4096);
    let feedback = json::parse(text.split_once("result:\n").unwrap().1).unwrap();
    assert_eq!(feedback.get("candidate"), Some(&proposal));
    let sources = feedback.get("cited_sources").unwrap().as_array().unwrap();
    assert_eq!(sources.len(), 2);
    assert_eq!(
        sources[0].get("eligible_proof"),
        Some(&Value::Array(vec![]))
    );
    assert_eq!(
        sources[1].get("call_history"),
        Some(&Value::string("history:1"))
    );
    assert!(
        sources[1]
            .get("eligible_proof")
            .unwrap()
            .as_array()
            .unwrap()
            .contains(&Value::string("change"))
    );
    assert_eq!(
        super::super::prepare(&candidate, "", &messages, 0, 4096).unwrap_err(),
        error
    );
}

#[test]
fn malformed_and_over_budget_proposals_have_bounded_nonexecuting_feedback() {
    let malformed = "{not JSON}";
    let text = validation_feedback(malformed, "Invalid JSON", "", &[], 0, 512);
    let value = json::parse(text.split_once("result:\n").unwrap().1).unwrap();
    assert_eq!(value.get("candidate"), Some(&Value::string(malformed)));
    assert_eq!(
        value.get("previous_context_preserved"),
        Some(&Value::Bool(true))
    );
    let large = "Untrusted candidate ".repeat(1000);
    let text = validation_feedback(&large, "Over budget", "", &[], 0, 512);
    assert!(text.len() < 512);
    let value = json::parse(text.split_once("result:\n").unwrap().1).unwrap();
    assert_eq!(value.get("candidate"), Some(&Value::Null));
    assert_eq!(
        value.get("candidate_bytes"),
        Some(&Value::number(large.len()))
    );
    assert_eq!(value.get("cited_sources"), Some(&Value::Array(vec![])));
}

#[test]
fn a_review_failure_does_not_hide_independent_proof_errors_in_other_entries() {
    let messages = vec![
        Value::object([
            ("role", Value::string("user")),
            ("content", Value::string("Keep data; build index")),
        ]),
        Value::object([
            ("role", Value::string("assistant")),
            ("content", Value::string("Index proposed")),
        ]),
        Value::object([
            ("role", Value::string("user")),
            ("content", Value::string("Cancel index; audit report")),
        ]),
    ];
    let mut old = json::parse(&super::super::fixture("Build index")).unwrap();
    if let Value::Object(fields) = &mut old {
        fields.insert(
            "constraints".into(),
            Value::Array(vec![Value::string("Keep data")]),
        );
        fields.insert(
            "remaining".into(),
            Value::Array(vec![Value::string("Build index")]),
        );
        fields.insert("reviewed_request_history".into(), Value::number(0));
    }
    let previous = old.encode();
    let mut candidate = json::parse(&super::super::fixture("Audit report")).unwrap();
    if let Value::Object(fields) = &mut candidate {
        fields.insert(
            "completed".into(),
            Value::Array(
                ["check", "change"]
                    .into_iter()
                    .map(|kind| {
                        Value::object([
                            ("description", Value::string("Claimed action")),
                            ("kind", Value::string(kind)),
                            ("evidence", Value::Array(vec![Value::string("history:1")])),
                        ])
                    })
                    .collect(),
            ),
        );
    }
    let original_messages = messages.clone();
    let candidate = candidate.encode();
    let error = super::super::prepare(&candidate, &previous, &messages, 1, 4096).unwrap_err();
    assert!(error.contains("requires explicit review"));
    let text = validation_feedback(&candidate, &error, &previous, &messages, 1, 4096);
    let feedback = json::parse(text.split_once("result:\n").unwrap().1).unwrap();
    let errors = feedback
        .get("validation_errors")
        .unwrap()
        .as_array()
        .unwrap();
    assert_eq!(errors.len(), 3);
    assert_eq!(errors[0].as_str(), Some(error.as_str()));
    assert!(
        errors[1]
            .as_str()
            .unwrap()
            .contains("completed[0] references history:1 (assistant)")
    );
    assert!(
        errors[2]
            .as_str()
            .unwrap()
            .contains("completed[1] references history:1 (assistant)")
    );
    assert_eq!(
        super::super::prepare(&candidate, &previous, &messages, 1, 4096).unwrap_err(),
        error
    );
    assert_eq!(messages, original_messages);
}

#[test]
fn invalid_pending_shape_does_not_hide_an_independent_assistant_proof_fault() {
    let messages = vec![
        Value::object([("role", Value::string("user"))]),
        Value::object([("role", Value::string("assistant"))]),
    ];
    for missing in [true, false] {
        let mut candidate = json::parse(&super::super::fixture("Audit")).unwrap();
        if let Value::Object(fields) = &mut candidate {
            fields.insert(
                "completed".into(),
                Value::Array(vec![Value::object([
                    ("description", Value::string("Claimed check")),
                    ("kind", Value::string("check")),
                    ("evidence", Value::Array(vec![Value::string("history:1")])),
                ])]),
            );
            if missing {
                fields.remove("remaining");
            } else {
                fields.insert(
                    "remaining".into(),
                    Value::Array(vec![Value::object([
                        ("description", Value::string("Audit later")),
                        ("evidence", Value::Array(vec![Value::string("history:0")])),
                    ])]),
                );
            }
        }
        let text = candidate.encode();
        let error = super::super::prepare(&text, "", &messages, 0, 4096).unwrap_err();
        assert!(error.contains("remaining"));
        let feedback = validation_feedback(&text, &error, "", &messages, 0, 4096);
        let value = json::parse(feedback.split_once("result:\n").unwrap().1).unwrap();
        let faults = value.get("validation_errors").unwrap().as_array().unwrap();
        assert_eq!(faults.len(), 2);
        assert!(
            faults[1]
                .as_str()
                .unwrap()
                .contains("completed[0] references history:1 (assistant)")
        );
        assert_eq!(
            super::super::prepare(&text, "", &messages, 0, 4096).unwrap_err(),
            error
        );
    }
}
