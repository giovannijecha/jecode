use super::*;

fn previous() -> Value {
    Value::object([
        (
            "constraints",
            Value::Array(vec![Value::string("Keep data exactly\r\n")]),
        ),
        (
            "remaining",
            Value::Array(vec![Value::string("Build index")]),
        ),
    ])
}

#[test]
fn explicit_resolution_ids_inside_descriptions_keep_native_proof_and_exact_identity() {
    let old = previous();
    let messages = vec![
        Value::object([
            ("role", Value::string("user")),
            ("content", Value::string("Build index; keep data")),
        ]),
        Value::object([
            ("role", Value::string("user")),
            ("content", Value::string("Cancel index; allow data changes")),
        ]),
    ];
    let mut old = old;
    if let Value::Object(fields) = &mut old {
        fields.insert("objective".into(), Value::string("Build index"));
        fields.insert("next_action".into(), Value::string("Build index"));
        fields.insert("completed".into(), Value::Array(vec![]));
        fields.insert("reviewed_request_history".into(), Value::number(0));
    }
    let mut proposed = json::parse(&super::super::fixture("Cancelled")).unwrap();
    if let Value::Object(fields) = &mut proposed {
        fields.insert(
            "resolved".into(),
            Value::Array(
                [
                    ("constraints", "Keep data exactly\r\n"),
                    ("remaining", "Build index"),
                ]
                .into_iter()
                .map(|(key, text)| {
                    Value::object([
                        (
                            "description",
                            Value::string(format!(
                                "Withdraw {}: original user cancelled this requirement.",
                                label(key, text)
                            )),
                        ),
                        ("kind", Value::string("decision")),
                        ("evidence", Value::Array(vec![Value::string("history:1")])),
                    ])
                })
                .collect(),
            ),
        );
    }
    let original = proposed.clone();
    let accepted =
        super::super::prepare(&proposed.encode(), &old.encode(), &messages, 1, 4096).unwrap();
    let accepted = json::parse(&accepted).unwrap();
    assert_eq!(accepted.get("constraints"), Some(&Value::Array(vec![])));
    assert!(
        !accepted
            .get("remaining")
            .unwrap()
            .as_array()
            .unwrap()
            .contains(&Value::string("Build index"))
    );
    assert_eq!(
        accepted.get("resolved").unwrap().as_array().unwrap()[0].get("description"),
        Some(&Value::string("Keep data exactly\r\n"))
    );
    // A recognized target is not new decision evidence.
    if let Value::Object(fields) = &mut proposed
        && let Some(Value::Array(entries)) = fields.get_mut("resolved")
    {
        for entry in entries {
            if let Value::Object(fields) = entry {
                fields.insert(
                    "evidence".into(),
                    Value::Array(vec![Value::string("history:0")]),
                );
            }
        }
    }
    assert!(super::super::prepare(&proposed.encode(), &old.encode(), &messages, 1, 4096).is_err());
    assert!(
        original.get("resolved").unwrap().as_array().unwrap()[0]
            .get("description")
            .unwrap()
            .as_str()
            .unwrap()
            .starts_with("Withdraw @constraints:")
    );
}

#[test]
fn ambiguous_unknown_or_extended_ids_cannot_select_an_old_item_by_guessing() {
    let old = previous();
    let first = label("constraints", "Keep data exactly\r\n");
    let second = label("remaining", "Build index");
    let mut ambiguous = Value::string(format!("Resolve {first} and {second}"));
    assert!(
        expand_resolution(&mut ambiguous, &old)
            .unwrap_err()
            .contains("ambiguous")
    );
    for text in [
        format!("Resolve {second}0"),
        format!("Resolve {second}_other"),
        "Withdraw the old index task".into(),
        "Resolve @remaining:h0000000000000000".into(),
    ] {
        let mut description = Value::string(&text);
        expand_resolution(&mut description, &old).unwrap();
        assert_eq!(description, Value::string(text));
    }
    let mut repeated = Value::string(format!("Resolve {second}; source target {second}"));
    expand_resolution(&mut repeated, &old).unwrap();
    assert_eq!(repeated, Value::string("Build index"));
}
