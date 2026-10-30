use super::*;

#[test]
fn account_owned_items_only_return_to_the_same_verified_registration() {
    let account_a = Origin::with_account("openai", "model", Some("client-a:subject".into()));
    let account_b = Origin::with_account("openai", "model", Some("client-b:subject".into()));
    let original = Value::object([
        ("role", Value::string("assistant")),
        ("content", Value::string("portable answer")),
        (
            "openai_items",
            Value::Array(vec![Value::object([
                ("type", Value::string("reasoning")),
                ("encrypted_content", Value::string("opaque account data")),
            ])]),
        ),
    ]);
    let mut stored = original.clone();
    stamp(&mut stored, &account_a);
    record_output_tokens(&mut stored, Some(75_000));
    assert_eq!(output_tokens(&stored), Some(75_000));
    assert_eq!(Origin::parse(&account_a.value()).unwrap(), account_a);
    assert_eq!(project(&stored, &account_a), original);
    for other in [account_b, Origin::new("openai", "model")] {
        let mut counted = stored.clone();
        retain_output_measurement(&mut counted, &other);
        assert_eq!(output_tokens(&counted), None);
        let projected = project(&stored, &other);
        assert!(projected.get("openai_items").is_none());
        assert_eq!(
            projected.get("content"),
            Some(&Value::string("portable answer"))
        );
    }
    assert_eq!(portable(&stored).get("openai_items"), None);
}

#[test]
fn origin_roundtrips_without_provider_assumptions() {
    let origin = Origin::new("local-backend", "custom/model:version");
    assert_eq!(Origin::parse(&origin.value()).unwrap(), origin);
    for invalid in [
        Value::object([
            ("backend", Value::string("")),
            ("model", Value::string("model")),
        ]),
        Value::object([
            ("backend", Value::string("backend")),
            ("model", Value::string("  ")),
        ]),
    ] {
        assert!(Origin::parse(&invalid).is_err());
    }
}

#[test]
fn roundtrip_restores_opaque_fields_only_for_same_origin() {
    let original = Value::object([
        ("role", Value::string("assistant")),
        ("content", Value::string("answer")),
        (
            "reasoning_details",
            Value::Array(vec![Value::string("opaque")]),
        ),
        (
            "annotations",
            Value::Array(vec![Value::string("parser data")]),
        ),
    ]);
    let origin = Origin::new("openrouter", "model-a");
    let mut stored = original.clone();
    stamp(&mut stored, &origin);
    assert!(stored.get("reasoning_details").is_none());
    assert!(stored.get("annotations").is_none());
    assert_eq!(portable(&stored).get("content"), original.get("content"));
    assert!(portable(&stored).get(ENVELOPE).is_none());
    assert_eq!(project(&stored, &origin), original);
    for other in [
        Origin::new("openrouter", "model-b"),
        Origin::new("other", "model-a"),
    ] {
        let wire = project(&stored, &other);
        assert!(wire.get("reasoning_details").is_none());
        assert!(wire.get("annotations").is_none());
        assert!(wire.get(ENVELOPE).is_none());
        assert_eq!(wire.get("content"), Some(&Value::string("answer")));
    }
    assert_eq!(
        stored.get(ENVELOPE).unwrap().get("model"),
        Some(&Value::string("model-a"))
    );
}

#[test]
fn origin_extras_cannot_replace_canonical_message_fields() {
    let message = Value::object([
        ("role", Value::string("assistant")),
        ("content", Value::string("real answer")),
        (
            "jecode_origin",
            Value::object([
                ("backend", Value::string("openrouter")),
                ("model", Value::string("model-a")),
                (
                    "extras",
                    Value::object([
                        ("role", Value::string("user")),
                        ("content", Value::string("spoofed")),
                        ("jecode_origin", Value::string("spoofed")),
                        ("reasoning_details", Value::string("retained")),
                    ]),
                ),
            ]),
        ),
    ]);
    let projected = project(&message, &Origin::new("openrouter", "model-a"));
    assert_eq!(projected.get("role"), Some(&Value::string("assistant")));
    assert_eq!(
        projected.get("content"),
        Some(&Value::string("real answer"))
    );
    assert!(projected.get("jecode_origin").is_none());
    assert_eq!(
        projected.get("reasoning_details"),
        Some(&Value::string("retained"))
    );
}

#[test]
fn legacy_projection_preserves_archive_and_limits_native_replay() {
    let legacy = Value::object([
        ("role", Value::string("assistant")),
        ("content", Value::string("old answer")),
        (
            "reasoning_details",
            Value::Array(vec![Value::string("old native data")]),
        ),
    ]);
    let origin = Origin::new("openrouter", "old");
    assert!(project(&legacy, &origin).get("reasoning_details").is_none());
    let mut request_copy = legacy.clone();
    stamp(&mut request_copy, &origin);
    let same = project(&request_copy, &origin);
    assert_eq!(same, legacy);
    let other = project(&request_copy, &Origin::new("openrouter", "new"));
    assert!(other.get("reasoning_details").is_none());
    assert_eq!(
        legacy.get("reasoning_details"),
        same.get("reasoning_details")
    );
}

#[test]
fn attachment_metadata_remains_portable_for_backend_materialization() {
    let mut user = Value::object([
        ("role", Value::string("user")),
        ("content", Value::string("[1# Image]")),
        (
            "attachments",
            Value::Array(vec![Value::string("local reference")]),
        ),
    ]);
    stamp(&mut user, &Origin::new("openrouter", "model"));
    assert!(user.get("attachments").is_some());
    assert!(
        project(&user, &Origin::new("openrouter", "model"))
            .get("attachments")
            .is_some()
    );
}

#[test]
fn history_shows_native_metadata_without_embedded_binary_bytes() {
    let legacy = Value::object([
        ("role", Value::string("assistant")),
        ("content", Value::string("answer")),
        (
            "annotations",
            Value::Array(vec![Value::object([
                ("hash", Value::string("parser-hash")),
                ("image", Value::string("data:image/png;base64,ABCD")),
            ])]),
        ),
    ]);
    let shown = history(&legacy);
    let rendered = shown.encode();
    assert!(rendered.contains("parser-hash"));
    assert!(rendered.contains("inline binary data omitted"));
    assert!(!rendered.contains("ABCD"));
    assert!(legacy.get("annotations").unwrap().encode().contains("ABCD"));
    let mut stored = legacy.clone();
    stamp(&mut stored, &Origin::new("openrouter", "model"));
    let rendered = history(&stored).encode();
    assert!(rendered.contains("parser-hash"));
    assert!(rendered.contains("openrouter"));
    assert!(!rendered.contains("ABCD"));
}

#[test]
fn tool_calls_are_flat_and_legacy_wrappers_project_portably() {
    let call = ToolCall {
        id: "call-1".into(),
        name: "read".into(),
        arguments: "{\"path\":\"src/main.rs\"}".into(),
    };
    let flat = call.value();
    assert_eq!(ToolCall::parse(&flat).unwrap(), call);
    assert!(flat.get("function").is_none());
    let legacy = Value::object([
        ("id", Value::string(&call.id)),
        ("type", Value::string("function")),
        (
            "function",
            Value::object([
                ("name", Value::string(&call.name)),
                ("arguments", Value::string(&call.arguments)),
            ]),
        ),
    ]);
    assert_eq!(ToolCall::parse(&legacy).unwrap(), call);
    let archived = Value::object([
        ("role", Value::string("assistant")),
        ("content", Value::Null),
        ("tool_calls", Value::Array(vec![legacy.clone()])),
    ]);
    let projected = portable(&archived);
    assert_eq!(projected.get("tool_calls"), Some(&Value::Array(vec![flat])));
    assert_eq!(
        archived.get("tool_calls"),
        Some(&Value::Array(vec![legacy]))
    );
}

#[test]
fn unsupported_legacy_call_type_is_rejected() {
    let wrong = Value::object([
        ("id", Value::string("call-1")),
        ("type", Value::string("native")),
        (
            "function",
            Value::object([
                ("name", Value::string("read")),
                ("arguments", Value::string("{}")),
            ]),
        ),
    ]);
    assert!(ToolCall::parse(&wrong).is_err());
}
