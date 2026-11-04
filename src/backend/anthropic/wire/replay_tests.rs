use super::*;

#[test]
fn native_replay_uses_executed_arguments_and_preserves_signed_thinking() {
    let original = r#"{"paths":"[\"a.rs\",\"b.rs\"]"}"#;
    let decoded = crate::tools::coerce::arguments("read", original).unwrap();
    let thinking = Value::object([
        ("type", Value::string("thinking")),
        ("thinking", Value::string("Inspect both files")),
        ("signature", Value::string("fixture-signature")),
    ]);
    let tool = Value::object([
        ("type", Value::string("tool_use")),
        ("id", Value::string("read-1")),
        ("name", Value::string("read")),
        ("input", crate::json::parse(original).unwrap()),
        ("provider_extension", Value::Bool(true)),
    ]);
    let mut message = Value::object([
        ("role", Value::string("assistant")),
        (
            "tool_calls",
            Value::Array(vec![
                ToolCall {
                    id: "read-1".into(),
                    name: "read".into(),
                    arguments: decoded.clone(),
                }
                .value(),
            ]),
        ),
        (
            "anthropic_blocks",
            Value::Array(vec![thinking.clone(), tool.clone()]),
        ),
    ]);
    let origin = crate::conversation::Origin::with_account("anthropic", "model", None);
    crate::conversation::stamp(&mut message, &origin);
    let replayed = assistant(&crate::conversation::project(&message, &origin)).unwrap();
    let blocks = replayed.get("content").and_then(Value::as_array).unwrap();
    assert_eq!(blocks[0], thinking);
    assert_eq!(
        blocks[1].get("input"),
        Some(&crate::json::parse(&decoded).unwrap())
    );
    assert_eq!(
        blocks[1].get("provider_extension"),
        Some(&Value::Bool(true))
    );
    let archived = crate::conversation::project(&message, &origin);
    assert_eq!(
        archived
            .get("anthropic_blocks")
            .and_then(Value::as_array)
            .unwrap()[1],
        tool
    );
}
