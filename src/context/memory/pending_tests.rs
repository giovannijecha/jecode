use super::*;

fn pending(description: &str, kind: &str) -> Value {
    Value::object([
        ("description", Value::string(description)),
        ("kind", Value::string(kind)),
    ])
}

fn candidate(items: Vec<Value>) -> Value {
    let mut value = json::parse(&fixture("Audit the report")).unwrap();
    if let Value::Object(fields) = &mut value {
        fields.insert("remaining".into(), Value::Array(items));
    }
    value
}

#[test]
fn pending_task_annotations_preserve_text_and_carry_prior_work_without_claiming_proof() {
    let previous = fixture("Audit the report");
    let description = "Verify records.tsv\r\nwithout repeating the inspection.";
    let proposed = candidate(vec![pending(description, "check")]);
    let original = proposed.clone();
    let prepared = prepare(&proposed.encode(), &previous, &[], 0, 8192).unwrap();
    let memory = json::parse(&prepared).unwrap();
    let remaining = memory.get("remaining").unwrap().as_array().unwrap();
    assert!(remaining.contains(&Value::string(description)));
    assert!(remaining.contains(&Value::string("Continue verification")));
    assert!(
        memory
            .get("completed")
            .unwrap()
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert_eq!(proposed, original);
}

#[test]
fn an_annotated_prior_label_expands_once_and_keeps_the_exact_owned_identity() {
    let previous = fixture("Audit the report");
    let labeled = json::parse(&prompt_view(&previous)).unwrap();
    let label = labeled.get("remaining").unwrap().as_array().unwrap()[0]
        .as_str()
        .unwrap();
    let proposed = candidate(vec![pending(label, "inspection")]);
    let prepared = prepare(&proposed.encode(), &previous, &[], 0, 8192).unwrap();
    let memory = json::parse(&prepared).unwrap();
    assert_eq!(
        memory.get("remaining").unwrap(),
        &Value::Array(vec![Value::string("Continue verification")])
    );
}

#[test]
fn pending_adaptation_cannot_accept_proof_fields_or_erase_missing_requirements() {
    let previous = fixture("Audit the report");
    let mut proof = pending("Verification complete", "check");
    if let Value::Object(fields) = &mut proof {
        fields.insert(
            "evidence".into(),
            Value::Array(vec![Value::string("history:7")]),
        );
    }
    let invalid = [
        proof,
        pending("Verify the report", "passed"),
        pending("  ", "check"),
        Value::object([("kind", Value::string("check"))]),
        Value::object([("description", Value::Bool(true))]),
    ];
    for item in invalid {
        let proposed = candidate(vec![pending("Inspect the data", "inspection"), item]);
        assert!(prepare(&proposed.encode(), &previous, &[], 0, 8192).is_err());
        assert_eq!(previous, fixture("Audit the report"));
    }
    let mut missing = candidate(vec![]);
    if let Value::Object(fields) = &mut missing {
        fields.remove("remaining");
    }
    assert!(
        prepare(&missing.encode(), &previous, &[], 0, 8192)
            .unwrap_err()
            .contains("requires remaining as an array")
    );
}

#[test]
fn a_pending_check_annotation_does_not_make_an_unproven_completion_valid() {
    let previous = fixture("Audit the report");
    let mut proposed = candidate(vec![pending("Verify the report", "check")]);
    if let Value::Object(fields) = &mut proposed {
        fields.insert(
            "completed".into(),
            Value::Array(vec![Value::object([
                ("description", Value::string("Continue verification")),
                ("kind", Value::string("check")),
                ("evidence", Value::Array(vec![Value::string("history:7")])),
            ])]),
        );
    }
    assert!(prepare(&proposed.encode(), &previous, &[], 0, 8192).is_err());
    assert_eq!(previous, fixture("Audit the report"));
}
