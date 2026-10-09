use super::*;
use crate::test_support::tool_call;

fn messages(check: bool, exit: usize) -> Vec<Value> {
    vec![
        Value::object([
            ("role", Value::string("system")),
            ("content", Value::string("Agent")),
        ]),
        Value::object([
            ("role", Value::string("user")),
            ("content", Value::string("Repair; preserve protected files")),
        ]),
        Value::object([
            ("role", Value::string("assistant")),
            (
                "tool_calls",
                Value::Array(vec![tool_call(
                    "verify",
                    "bash",
                    Value::object([
                        ("command", Value::string("run-tests")),
                        ("check", Value::Bool(check)),
                    ]),
                )]),
            ),
        ]),
        Value::object([
            ("role", Value::string("tool")),
            ("tool_call_id", Value::string("verify")),
            (
                "content",
                Value::string(
                    Value::object([
                        ("exit_code", Value::number(exit)),
                        ("check", Value::Bool(check)),
                        (
                            "check_status",
                            Value::string(if check && exit == 0 {
                                "passed"
                            } else {
                                "failed"
                            }),
                        ),
                    ])
                    .encode(),
                ),
            ),
        ]),
    ]
}

fn state(constraints: &[&str], remaining: &[&str], completed: Vec<Value>) -> String {
    Value::object([
        ("objective", Value::string("Repair the parser")),
        (
            "constraints",
            Value::Array(
                constraints
                    .iter()
                    .map(|text| Value::string(*text))
                    .collect(),
            ),
        ),
        ("completed", Value::Array(completed)),
        (
            "remaining",
            Value::Array(remaining.iter().map(|text| Value::string(*text)).collect()),
        ),
        (
            "next_action",
            Value::string("Continue from the unfinished work"),
        ),
    ])
    .encode()
}

fn done(description: &str) -> Value {
    Value::object([
        ("description", Value::string(description)),
        ("kind", Value::string("check")),
        ("evidence", Value::Array(vec![Value::string("history:3")])),
    ])
}

#[test]
fn a_generic_or_incomplete_summary_cannot_replace_operational_memory() {
    for summary in [
        "Implementation is complete. Final checks remain.",
        "{}",
        r#"{"objective":"done","constraints":[],"completed":[],"remaining":[]}"#,
    ] {
        assert!(validate(summary, "", &messages(true, 0), 1, 4096).is_err());
    }
}

#[test]
fn constraints_and_unfinished_work_cannot_disappear_without_evidence() {
    let previous = state(
        &["Preserve protected files"],
        &["Run regression tests"],
        vec![],
    );
    let lost_constraint = state(&[], &["Run regression tests"], vec![]);
    assert!(
        validate(&lost_constraint, &previous, &messages(true, 0), 2, 4096)
            .unwrap_err()
            .contains("constraint disappeared")
    );
    let lost_work = state(&["Preserve protected files"], &[], vec![]);
    assert!(
        validate(&lost_work, &previous, &messages(true, 0), 2, 4096)
            .unwrap_err()
            .contains("unfinished item disappeared")
    );
    let resolved = state(
        &["Preserve protected files"],
        &[],
        vec![done("Run regression tests")],
    );
    assert!(validate(&resolved, &previous, &messages(true, 0), 2, 4096).is_ok());
}

#[test]
fn a_failed_or_ordinary_shell_command_cannot_prove_a_passed_check() {
    let summary = state(&[], &[], vec![done("Run regression tests")]);
    assert!(validate(&summary, "", &messages(true, 101), 1, 4096).is_err());
    assert!(validate(&summary, "", &messages(false, 0), 1, 4096).is_err());
    assert!(validate(&summary, "", &messages(true, 0), 1, 4096).is_ok());
    assert!(validate(&summary, "", &messages(true, 0)[..3], 1, 4096).is_err());
}

#[test]
fn supporting_reads_need_primary_proof_and_cannot_resolve_work_with_an_old_check() {
    let mut messages = messages(true, 0);
    for result in [
        Value::object([("content", Value::string("source context"))]),
        Value::object([("bytes_written", Value::number(42))]),
    ] {
        messages.push(Value::object([
            ("role", Value::string("tool")),
            ("content", Value::string(result.encode())),
        ]));
    }
    let previous = state(&[], &["Run regression tests"], vec![]);
    for (kind, proof) in [("check", "history:3"), ("change", "history:5")] {
        let mut entry = done("Run regression tests");
        if let Value::Object(fields) = &mut entry {
            fields.insert("kind".into(), Value::string(kind));
            fields.insert(
                "evidence".into(),
                Value::Array(vec![Value::string("history:4")]),
            );
        }
        assert!(
            validate(
                &state(&[], &[], vec![entry.clone()]),
                "",
                &messages,
                4,
                4096
            )
            .is_err()
        );
        if let Value::Object(fields) = &mut entry {
            fields.insert(
                "evidence".into(),
                Value::Array(vec![Value::string("history:4"), Value::string(proof)]),
            );
        }
        let candidate = state(&[], &[], vec![entry]);
        assert!(validate(&candidate, "", &messages, 4, 4096).is_ok());
        assert_eq!(
            validate(&candidate, &previous, &messages, 4, 4096).is_ok(),
            kind == "change"
        );
    }
}

#[test]
fn prior_proof_is_preserved_and_old_checks_do_not_resolve_new_work() {
    let previous = state(
        &[],
        &["Run regression tests"],
        vec![done("Earlier verification")],
    );
    let dropped = state(&[], &["Run regression tests"], vec![]);
    assert!(
        validate(&dropped, &previous, &messages(true, 0), 4, 4096)
            .unwrap_err()
            .contains("completed evidence disappeared")
    );
    let old_proof = state(&[], &[], vec![done("Run regression tests")]);
    assert!(
        validate(&old_proof, &previous, &messages(true, 0), 4, 4096)
            .unwrap_err()
            .contains("unfinished item disappeared")
    );
}

#[test]
fn the_harness_carries_omitted_constraints_pending_work_and_previous_evidence() {
    let previous = state(
        &["Preserve protected files"],
        &["Run regression tests"],
        vec![done("Earlier verification")],
    );
    let rewritten = state(&["Do not modify protected files"], &[], vec![]);
    let prepared = prepare(&rewritten, &previous, &messages(true, 0), 4, 4096).unwrap();
    let value = json::parse(&prepared).unwrap();
    assert!(
        strings(&value, "constraints")
            .unwrap()
            .contains(&"Preserve protected files")
    );
    assert!(
        strings(&value, "remaining")
            .unwrap()
            .contains(&"Run regression tests")
    );
    assert!(
        value
            .get("completed")
            .unwrap()
            .encode()
            .contains("history:3")
    );
    let resolved = state(
        &["Preserve protected files"],
        &[],
        vec![done("Run regression tests")],
    );
    let prepared = prepare(&resolved, &previous, &messages(true, 0), 2, 4096).unwrap();
    assert!(
        strings(&json::parse(&prepared).unwrap(), "remaining")
            .unwrap()
            .is_empty()
    );
}

#[test]
fn reusing_evidence_cannot_replace_the_identity_of_completed_work() {
    let earlier = done("Earlier verification: first diagnostic already executed");
    let previous = state(&[], &[], vec![earlier.clone()]);
    for mut replacement in [done("Different work"), earlier.clone()] {
        if replacement == earlier
            && let Value::Object(fields) = &mut replacement
        {
            fields.insert("kind".into(), Value::string("inspection"));
        }
        let candidate = state(&[], &[], vec![replacement]);
        assert!(validate(&candidate, &previous, &messages(true, 0), 4, 4096).is_err());
        let prepared = prepare(&candidate, &previous, &messages(true, 0), 4, 4096).unwrap();
        let prepared = json::parse(&prepared).unwrap();
        assert!(
            prepared
                .get("completed")
                .unwrap()
                .as_array()
                .unwrap()
                .contains(&earlier)
        );
    }
}

#[test]
fn a_known_failed_inspection_is_retained_without_resolving_unfinished_work() {
    let previous = state(&[], &["Run regression tests"], vec![]);
    let mut inspected = done("Run regression tests");
    if let Value::Object(fields) = &mut inspected {
        fields.insert("kind".into(), Value::string("inspection"));
    }
    let candidate = state(&[], &[], vec![inspected]);
    let prepared = prepare(&candidate, &previous, &messages(true, 101), 2, 4096).unwrap();
    let value = json::parse(&prepared).unwrap();
    assert!(
        value
            .get("completed")
            .unwrap()
            .encode()
            .contains("history:3")
    );
    assert!(
        strings(&value, "remaining")
            .unwrap()
            .contains(&"Run regression tests")
    );
    assert!(
        validate(
            &state(&[], &[], vec![done("Run regression tests")]),
            &previous,
            &messages(true, 101),
            2,
            4096
        )
        .is_err()
    );
}

#[test]
fn verbose_completed_descriptions_shrink_while_constraints_work_and_proof_survive() {
    let mut completed = done(&"Verified detailed diagnostic Ω 日本; ".repeat(100));
    if let Value::Object(fields) = &mut completed {
        fields.insert("kind".into(), Value::string("inspection"));
    }
    let previous = state(
        &["Preserve protected files"],
        &["Run regression tests"],
        vec![completed],
    );
    let prepared = prepare(&previous, &previous, &messages(true, 0), 4, 1500).unwrap();
    assert_eq!(prepared, previous);
    assert!(active_view(&prepared, 1500).len() <= 1500);
    let value = json::parse(&prepared).unwrap();
    assert!(
        strings(&value, "constraints")
            .unwrap()
            .contains(&"Preserve protected files")
    );
    assert!(
        strings(&value, "remaining")
            .unwrap()
            .contains(&"Run regression tests")
    );
    assert!(
        value
            .get("completed")
            .unwrap()
            .encode()
            .contains("history:3")
    );
    assert!(active_view(&prepared, 1500).contains("see evidence"));
}

#[test]
fn scoped_references_resolve_prior_work_without_retyping_and_allow_a_new_user_decision() {
    let previous = state(
        &["Preserve protected files"],
        &["Run regression tests"],
        vec![],
    );
    let remaining_label = references::label("remaining", "Run regression tests");
    let constraint_label = references::label("constraints", "Preserve protected files");
    let mut completed = done(&remaining_label);
    let resolved = state(
        &[&format!("{constraint_label} paraphrased text")],
        &[],
        vec![completed.clone()],
    );
    let prepared = prepare(&resolved, &previous, &messages(true, 0), 2, 4096).unwrap();
    let value = json::parse(&prepared).unwrap();
    assert!(strings(&value, "remaining").unwrap().is_empty());
    assert_eq!(
        strings(&value, "constraints").unwrap(),
        vec!["Preserve protected files"]
    );
    let mut history = messages(true, 0);
    history.push(Value::object([
        ("role", Value::string("user")),
        (
            "content",
            Value::string("You may change the protected file"),
        ),
    ]));
    if let Value::Object(fields) = &mut completed {
        fields.insert("description".into(), Value::string(&constraint_label));
        fields.insert("kind".into(), Value::string("decision"));
        fields.insert(
            "evidence".into(),
            Value::Array(vec![Value::string("history:4")]),
        );
    }
    let mut candidate = json::parse(&state(&[], &[&remaining_label], vec![])).unwrap();
    if let Value::Object(fields) = &mut candidate {
        fields.insert("resolved".into(), Value::Array(vec![completed]));
    }
    let prepared = prepare(&candidate.encode(), &previous, &history, 4, 4096).unwrap();
    assert!(
        strings(&json::parse(&prepared).unwrap(), "constraints")
            .unwrap()
            .is_empty()
    );
    assert!(prompt_view(&previous).contains("@remaining:h"));
}

#[test]
fn unusable_labels_cannot_approve_prior_work_and_preserve_explicit_uncertainty() {
    let previous = state(
        &["Preserve tests exactly"],
        &["Implement filtering", "Run regression tests"],
        vec![],
    );
    let mut bad_resolution = done("@constraints:h0000000000000000");
    if let Value::Object(fields) = &mut bad_resolution {
        fields.insert("kind".into(), Value::string("decision"));
        fields.insert(
            "evidence".into(),
            Value::Array(vec![Value::string("history:1")]),
        );
    }
    let mut candidate = json::parse(&state(
        &["@constraints:7"],
        &["@remaining:7"],
        vec![done("@remaining:0"), done("Verified new check")],
    ))
    .unwrap();
    if let Value::Object(fields) = &mut candidate {
        fields.insert("resolved".into(), Value::Array(vec![bad_resolution]));
    }
    let (prepared, notices) =
        prepare_with_notices(&candidate.encode(), &previous, &messages(true, 0), 2, 4096).unwrap();
    let prepared = json::parse(&prepared).unwrap();
    assert_eq!(
        strings(&prepared, "constraints").unwrap(),
        vec!["Preserve tests exactly"]
    );
    let remaining = strings(&prepared, "remaining").unwrap();
    assert!(remaining.contains(&"Implement filtering"));
    assert!(remaining.contains(&"Run regression tests"));
    assert!(
        remaining
            .iter()
            .any(|item| item.contains("Reconcile unresolved"))
    );
    assert_eq!(
        prepared.get("completed").unwrap().as_array().unwrap().len(),
        1
    );
    assert_eq!(notices.len(), 4);
    let invalid_proof = state(&[], &[], vec![done("Claims a passed check")]);
    assert!(prepare_with_notices(&invalid_proof, &previous, &messages(true, 1), 2, 4096).is_err());
}

#[test]
fn content_labels_do_not_change_when_other_remaining_items_are_removed_or_reordered() {
    let original = "Run regression tests and preserve all acceptance cases";
    let first = state(
        &[],
        &["Implement change", original, "Report results"],
        vec![],
    );
    let later = state(&[], &["Report results", original], vec![]);
    let label = references::label("remaining", original);
    assert!(prompt_view(&first).contains(&label));
    assert!(prompt_view(&later).contains(&label));
    let mut candidate = json::parse(&state(&[], &[&label], vec![])).unwrap();
    references::expand(&mut candidate, &json::parse(&later).unwrap()).unwrap();
    assert_eq!(strings(&candidate, "remaining").unwrap(), vec![original]);
    let removed = state(&[], &["Report results"], vec![]);
    let mut candidate = json::parse(&state(&[], &[&label], vec![])).unwrap();
    assert!(references::expand(&mut candidate, &json::parse(&removed).unwrap()).is_err());
}

#[test]
fn small_contexts_can_shorten_completed_labels_below_160_bytes_without_losing_proof() {
    let completed = (0..10)
        .map(|index| {
            let mut entry = done(&format!(
                "Inspection {index}: {}",
                "detailed evidence ".repeat(30)
            ));
            if let Value::Object(fields) = &mut entry {
                fields.insert("kind".into(), Value::string("inspection"));
            }
            entry
        })
        .collect();
    let previous = state(
        &["Preserve protected files exactly"],
        &["Run regression tests"],
        completed,
    );
    let prepared = prepare(&previous, &previous, &messages(true, 0), 4, 1800).unwrap();
    assert_eq!(prepared, previous);
    let value = json::parse(&active_view(&prepared, 1800)).unwrap();
    assert_eq!(
        strings(&value, "constraints").unwrap(),
        vec!["Preserve protected files exactly"]
    );
    assert_eq!(
        strings(&value, "remaining").unwrap(),
        vec!["Run regression tests"]
    );
    let completed = value.get("completed").unwrap().as_array().unwrap();
    assert_eq!(completed.len(), 10);
    assert!(completed.iter().all(|entry| {
        entry
            .get("description")
            .and_then(Value::as_str)
            .unwrap()
            .len()
            <= 96
    }));
    assert!(
        completed
            .iter()
            .all(|entry| entry.get("evidence").unwrap().as_array().unwrap()
                == [Value::string("history:3")])
    );
}
