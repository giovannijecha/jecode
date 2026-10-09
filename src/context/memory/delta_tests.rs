use super::*;

#[test]
fn omitted_completed_additions_preserve_known_proof_without_weakening_ledger_validation() {
    let messages = [Value::object([
        ("role", Value::string("user")),
        ("content", Value::string("Preserve all original tests.")),
    ])];
    let proof = Value::object([
        (
            "description",
            Value::string("Original preservation requirement Ω"),
        ),
        ("kind", Value::string("decision")),
        ("evidence", Value::Array(vec![Value::string("history:0")])),
    ]);
    let mut previous = json::parse(&fixture("Continue the original task")).unwrap();
    if let Value::Object(fields) = &mut previous {
        fields.insert("completed".into(), Value::Array(vec![proof.clone()]));
        fields.insert(
            "constraints".into(),
            Value::Array(vec![Value::string("Preserve all original tests.")]),
        );
    }
    let previous = previous.encode();
    let mut candidate = json::parse(&fixture("Continue the original task")).unwrap();
    if let Value::Object(fields) = &mut candidate {
        fields.remove("completed");
    }
    let omitted = candidate.encode();
    assert!(validate(&omitted, &previous, &messages, 1, 4096).is_err());
    let assembled = prepare(&omitted, &previous, &messages, 1, 4096).unwrap();
    let ledger = json::parse(&assembled).unwrap();
    assert_eq!(ledger.get("completed"), Some(&Value::Array(vec![proof])));
    assert!(assembled.contains("Preserve all original tests."));
    assert!(assembled.contains("Continue verification"));
    assert!(validate(&assembled, &previous, &messages, 1, 4096).is_ok());
    assert!(prepare(&omitted, "", &messages, 1, 4096).is_err());
    if let Value::Object(fields) = &mut candidate {
        fields.insert("completed".into(), Value::string("unchanged"));
    }
    assert!(prepare(&candidate.encode(), &previous, &messages, 1, 4096).is_err());
}

#[test]
fn malformed_entries_identify_the_exact_field_without_accepting_unsupported_proof() {
    let messages = [
        Value::object([("role", Value::string("user"))]),
        Value::object([("role", Value::string("assistant"))]),
    ];
    let entry = |description: &str, kind: &str, evidence: Vec<Value>| {
        Value::object([
            ("description", Value::string(description)),
            ("kind", Value::string(kind)),
            ("evidence", Value::Array(evidence)),
        ])
    };
    let valid = entry(
        "User instruction",
        "decision",
        vec![Value::string("history:0")],
    );
    let cases = [
        (
            Value::string("@remaining:h0123456789abcdef"),
            "must be an object",
        ),
        (
            entry("", "decision", vec![Value::string("history:0")]),
            "nonempty description",
        ),
        (
            entry("Claim", "done", vec![Value::string("history:0")]),
            ".kind must be",
        ),
        (entry("Claim", "decision", vec![]), ".evidence must be"),
        (
            entry("Claim", "decision", vec![Value::string("history:99")]),
            "nonexistent history:99",
        ),
        (
            entry("Claim", "decision", vec![Value::string("history:1")]),
            "history:1 is assistant",
        ),
    ];
    for key in ["completed", "resolved"] {
        for (invalid, expected) in &cases {
            let candidate =
                Value::object([(key, Value::Array(vec![valid.clone(), invalid.clone()]))]);
            let error = entries(&candidate, key, &messages).unwrap_err();
            assert!(error.contains(&format!("{key}[1]")), "{error}");
            assert!(error.contains(expected), "{error}");
        }
    }
}
