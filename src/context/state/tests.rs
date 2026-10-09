use super::*;
use crate::test_support::tool_call;

#[test]
fn stale_memory_and_executed_changes_keep_their_original_request_boundary() {
    let messages = vec![
        Value::object([("role", Value::string("system"))]),
        Value::object([
            ("role", Value::string("user")),
            ("content", Value::string("Build index")),
        ]),
        Value::object([
            ("role", Value::string("assistant")),
            (
                "tool_calls",
                Value::Array(vec![tool_call(
                    "draft",
                    "write",
                    Value::object([("path", Value::string("draft.txt"))]),
                )]),
            ),
        ]),
        Value::object([
            ("role", Value::string("user")),
            ("content", Value::string("Cancel index; audit only")),
        ]),
        Value::object([
            ("role", Value::string("tool")),
            ("tool_call_id", Value::string("draft")),
            (
                "content",
                Value::string(Value::object([("bytes_written", Value::number(4))]).encode()),
            ),
        ]),
    ];
    let mut memory = json::parse(&memory::fixture("Build index")).unwrap();
    if let Value::Object(fields) = &mut memory {
        fields.insert("reviewed_request_history".into(), Value::number(1));
    }
    let mut context = Context {
        from: 3,
        summary: memory.encode(),
        ..Context::default()
    };
    context.evidence.record(
        4,
        "write",
        &Value::object([("path", Value::string("draft.txt"))]),
        &Value::object([("bytes_written", Value::number(4))]),
    );
    let original_context = context.value();
    let original_messages = messages.clone();
    let view = context.project(&messages, 0);
    let text = view[1].get("content").unwrap().as_str().unwrap();
    let state = json::parse(text.split_once("state:\n").unwrap().1).unwrap();
    assert_eq!(
        state.get("latest_user_request"),
        Some(&Value::string("history:3"))
    );
    assert_eq!(
        state.get("memory_reviewed_request"),
        Some(&Value::string("history:1"))
    );
    assert_eq!(
        state.get("memory_proves_execution"),
        Some(&Value::Bool(false))
    );
    let source = state
        .get("execution_facts")
        .unwrap()
        .get("last_tool")
        .unwrap()
        .get("source")
        .unwrap();
    assert_eq!(
        source.get("request_history"),
        Some(&Value::string("history:1"))
    );
    assert_eq!(
        source.get("call_history"),
        Some(&Value::string("history:2"))
    );
    assert!(
        view.iter()
            .any(|message| message.get("content").and_then(Value::as_str)
                == Some("Cancel index; audit only"))
    );
    assert_eq!(context.value(), original_context);
    assert_eq!(messages, original_messages);
}

#[test]
fn unreviewed_memory_does_not_claim_a_current_request_or_drop_legacy_text() {
    let context = Context {
        summary: "Saved old task without a request stamp".into(),
        ..Context::default()
    };
    let messages = vec![Value::object([("role", Value::string("system"))])];
    let text = context.state_view(&messages);
    let state = json::parse(text.split_once("state:\n").unwrap().1).unwrap();
    assert_eq!(state.get("latest_user_request"), Some(&Value::Null));
    assert_eq!(state.get("memory_reviewed_request"), Some(&Value::Null));
    assert_eq!(state.get("memory"), Some(&Value::string(&context.summary)));
    assert_eq!(
        state.get("complete_memory"),
        Some(&Value::string("history:memory"))
    );
}
