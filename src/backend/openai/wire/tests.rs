use super::*;

#[test]
fn visible_catalog_keeps_order_and_unknown_capabilities() {
    let response = crate::json::parse(r#"{"models":[{"slug":"b","display_name":"Bee","visibility":"list"},{"slug":"hidden","visibility":"hidden"},{"slug":"a","visibility":"list"},{"slug":"b","visibility":"list"}]}"#).unwrap();
    let models = super::super::metadata::parse(&response).unwrap().models;
    assert_eq!(
        models
            .iter()
            .map(|model| model.id.as_str())
            .collect::<Vec<_>>(),
        ["b", "a"]
    );
    assert_eq!(models[0].name, "Bee");
    assert!(models[0].prompt_price.is_none());
}

#[test]
fn request_replays_native_items_and_canonical_tool_output() {
    let messages = vec![
        Value::object([
            ("role", Value::string("system")),
            ("content", Value::string("Guide")),
        ]),
        Value::object([
            ("role", Value::string("user")),
            ("content", Value::string("Hi")),
        ]),
        Value::object([
            ("role", Value::string("assistant")),
            ("content", Value::string("")),
            (
                "openai_items",
                Value::Array(vec![
                    Value::object([
                        ("type", Value::string("reasoning")),
                        ("encrypted_content", Value::string("opaque")),
                    ]),
                    Value::object([
                        ("type", Value::string("function_call")),
                        ("call_id", Value::string("c")),
                        ("name", Value::string("read")),
                        (
                            "arguments",
                            Value::string(r#"{"path":"src/main.rs","work":"Inspect source"}"#),
                        ),
                    ]),
                ]),
            ),
        ]),
        Value::object([
            ("role", Value::string("tool")),
            ("tool_call_id", Value::string("c")),
            ("content", Value::string("done")),
        ]),
    ];
    let request = request("model", Effort::High, &messages, true, None).unwrap();
    assert_eq!(request.get("store"), Some(&Value::Bool(false)));
    assert_eq!(request.get("stream"), Some(&Value::Bool(true)));
    assert_eq!(
        request.get("instructions").and_then(Value::as_str),
        Some("Guide")
    );
    let input = request.get("input").unwrap().as_array().unwrap();
    assert_eq!(
        input[1].get("encrypted_content").and_then(Value::as_str),
        Some("opaque")
    );
    assert_eq!(
        input[2].get("arguments").and_then(Value::as_str),
        Some(r#"{"path":"src/main.rs","work":"Inspect source"}"#)
    );
    assert_eq!(
        input[3].get("type").and_then(Value::as_str),
        Some("function_call_output")
    );
    assert!(request.get("previous_response_id").is_none());
    assert!(request.get("max_output_tokens").is_none());
    assert_eq!(
        request
            .get("reasoning")
            .and_then(|reasoning| reasoning.get("effort"))
            .and_then(Value::as_str),
        Some("high")
    );
    assert_eq!(
        request.get("tools").and_then(Value::as_array).unwrap()[0]
            .get("type")
            .and_then(Value::as_str),
        Some("namespace")
    );
    let read = &request.get("tools").and_then(Value::as_array).unwrap()[0]
        .get("tools")
        .and_then(Value::as_array)
        .unwrap()[0];
    assert_eq!(read.get("strict"), Some(&Value::Bool(false)));
    assert_eq!(
        read.get("parameters")
            .and_then(|parameters| parameters.get("properties"))
            .and_then(|properties| properties.get("work"))
            .and_then(|work| work.get("type"))
            .and_then(Value::as_str),
        Some("string")
    );
}

#[test]
fn attachment_marker_becomes_manifest_and_local_reference() {
    let directory = crate::test_support::Directory::new();
    let pool = Pool::new(directory.path().join("attachments"));
    let attachment = pool.import_bytes("notes.txt", b"hello").unwrap();
    let message = Value::object([
        ("role", Value::string("user")),
        (
            "content",
            Value::string(format!("Read this {}", attachments::MARKER)),
        ),
        ("attachments", Value::Array(vec![attachment.value()])),
    ]);
    let request = request("model", Effort::Default, &[message], false, Some(&pool)).unwrap();
    let input = request.get("input").and_then(Value::as_array).unwrap();
    let parts = input[0].get("content").and_then(Value::as_array).unwrap();
    let text = parts[0].get("text").and_then(Value::as_str).unwrap();
    assert!(text.contains(&attachment.reference()));
    assert!(!text.contains(attachments::MARKER));
}

#[test]
fn pdf_inline_limit_retains_a_local_reference_above_the_boundary() {
    let directory = crate::test_support::Directory::new();
    let pool = Pool::new(directory.path().join("attachments"));
    let mut pdf = pool
        .import_bytes("report.pdf", b"%PDF-1.4\n%%EOF\n")
        .unwrap();
    pdf.size = attachments::PDF_INLINE_BYTES;
    let message = |pdf: &attachments::Attachment| {
        attachments::Prompt::new(attachments::MARKER.to_string(), vec![pdf.clone()]).message()
    };
    let at_limit = request(
        "model",
        Effort::Default,
        &[message(&pdf)],
        false,
        Some(&pool),
    )
    .unwrap();
    let input = at_limit.get("input").and_then(Value::as_array).unwrap();
    let parts = input[0].get("content").and_then(Value::as_array).unwrap();
    assert_eq!(
        parts[1].get("type").and_then(Value::as_str),
        Some("input_file")
    );
    let data = parts[1]
        .get("file_data")
        .and_then(Value::as_str)
        .unwrap()
        .strip_prefix("data:application/pdf;base64,")
        .unwrap();
    assert_eq!(
        attachments::base64::decode(data).unwrap(),
        b"%PDF-1.4\n%%EOF\n"
    );
    pdf.size += 1;
    let above = request(
        "model",
        Effort::Default,
        &[message(&pdf)],
        false,
        Some(&pool),
    )
    .unwrap();
    let input = above.get("input").and_then(Value::as_array).unwrap();
    let parts = input[0].get("content").and_then(Value::as_array).unwrap();
    assert_eq!(parts.len(), 1);
    let manifest = parts[0].get("text").and_then(Value::as_str).unwrap();
    assert!(manifest.contains("PDF not inlined; read the local reference"));
    assert!(manifest.contains(&pdf.reference()));
}

#[test]
fn opaque_reasoning_is_not_replayed_for_another_account() {
    let mut message = Value::object([
        ("role", Value::string("assistant")),
        ("content", Value::string("Done")),
        (
            "openai_items",
            Value::Array(vec![Value::object([
                ("type", Value::string("reasoning")),
                ("encrypted_content", Value::string("account-a-secret")),
            ])]),
        ),
    ]);
    let account_a = crate::conversation::Origin::with_account("openai", "model", Some("a".into()));
    let account_b = crate::conversation::Origin::with_account("openai", "model", Some("b".into()));
    crate::conversation::stamp(&mut message, &account_a);
    let projected = crate::conversation::project(&message, &account_b);
    let foreign_request = request("model", Effort::Default, &[projected], false, None).unwrap();
    assert!(!foreign_request.encode().contains("account-a-secret"));
    let native = crate::conversation::project(&message, &account_a);
    assert!(
        request("model", Effort::Default, &[native], false, None)
            .unwrap()
            .encode()
            .contains("account-a-secret")
    );
}
