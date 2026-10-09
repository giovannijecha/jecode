use super::*;

#[test]
fn response_interface_distinguishes_pending_strings_from_evidence_objects() {
    let schema = json::parse(RESPONSE_SCHEMA).unwrap();
    let properties = schema.get("properties").unwrap();
    {
        let key = "constraints";
        assert_eq!(
            properties
                .get(key)
                .unwrap()
                .get("items")
                .unwrap()
                .get("type")
                .and_then(Value::as_str),
            Some("string")
        );
        assert!(
            schema
                .get("required")
                .unwrap()
                .as_array()
                .unwrap()
                .contains(&Value::string(key))
        );
    }
    let pending = properties
        .get("remaining")
        .unwrap()
        .get("items")
        .unwrap()
        .get("oneOf")
        .unwrap()
        .as_array()
        .unwrap();
    assert_eq!(
        pending[0].get("type").and_then(Value::as_str),
        Some("string")
    );
    assert_eq!(
        pending[1].get("$ref").and_then(Value::as_str),
        Some("#/$defs/pending")
    );
    for key in ["completed", "resolved"] {
        assert_eq!(
            properties
                .get(key)
                .unwrap()
                .get("items")
                .unwrap()
                .get("$ref")
                .and_then(Value::as_str),
            Some("#/$defs/proof")
        );
    }
    assert!(prompt().contains(RESPONSE_SCHEMA));
}

#[test]
fn malformed_pending_work_reports_its_exact_position_without_retyping_it() {
    let mut candidate = json::parse(&fixture("Audit the report")).unwrap();
    if let Value::Object(fields) = &mut candidate {
        fields.insert(
            "remaining".into(),
            Value::Array(vec![
                Value::string("Inspect the report"),
                Value::object([
                    ("description", Value::string("Verify the data")),
                    ("kind", Value::string("check")),
                    ("evidence", Value::Array(vec![Value::string("history:7")])),
                ]),
            ]),
        );
    }
    let original = candidate.clone();
    assert_eq!(
        prepare(&candidate.encode(), "", &[], 0, 4096).unwrap_err(),
        "Continuity memory remaining[1].evidence is not supported for pending work"
    );
    assert_eq!(candidate, original);
}
