use super::*;

#[test]
fn native_replay_keeps_received_tool_input_while_canonical_call_holds_executed_arguments() {
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
    let later = Value::object([
        ("type", Value::string("redacted_thinking")),
        ("data", Value::string("fixture-redacted")),
    ]);
    let native = vec![thinking, tool, later];
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
        ("anthropic_blocks", Value::Array(native.clone())),
    ]);
    let origin = crate::conversation::Origin::with_account("anthropic", "model", None);
    crate::conversation::stamp(&mut message, &origin);
    let projected = crate::conversation::project(&message, &origin);
    let replayed = assistant(&projected).unwrap();
    assert_eq!(replayed.get("content"), Some(&Value::Array(native)));
    let canonical = ToolCall::parse(&projected.get("tool_calls").unwrap().as_array().unwrap()[0]);
    assert_eq!(canonical.unwrap().arguments, decoded);
}
