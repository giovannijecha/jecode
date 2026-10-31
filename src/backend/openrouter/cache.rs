//! Explicit cache boundaries for providers that accept chat content markers.

use crate::json::Value;

pub(super) fn supports_openai(model: &str) -> bool {
    let model = model.strip_prefix('~').unwrap_or(model);
    ["openai/gpt-5.6", "openai/gpt-6"].iter().any(|known| {
        model == *known
            || model
                .strip_prefix(known)
                .is_some_and(|suffix| suffix.starts_with('-') || suffix.starts_with('.'))
    })
}

pub(super) fn mark_recent(messages: &mut [Value]) {
    let mut marked = 0;
    let mut inside_tool_batch = false;
    for message in messages.iter_mut().rev() {
        let tool = message.get("role").and_then(Value::as_str) == Some("tool");
        if tool && inside_tool_batch {
            continue;
        }
        inside_tool_batch = tool;
        if mark(message) {
            marked += 1;
            if marked == 4 {
                break;
            }
        }
    }
}

fn mark(message: &mut Value) -> bool {
    let Value::Object(fields) = message else {
        return false;
    };
    if !matches!(
        fields.get("role").and_then(Value::as_str),
        Some("user" | "system" | "developer" | "tool")
    ) {
        return false;
    }
    let Some(content) = fields.get_mut("content") else {
        return false;
    };
    let marker = Value::object([("type", Value::string("ephemeral"))]);
    match content {
        Value::String(text) if !text.is_empty() => {
            let text = text.clone();
            *content = Value::Array(vec![Value::object([
                ("type", Value::string("text")),
                ("text", Value::string(text)),
                ("cache_control", marker),
            ])]);
            true
        }
        Value::Array(blocks) => {
            let Some(Value::Object(block)) = blocks.iter_mut().rev().find(|block| {
                block.get("type").and_then(Value::as_str) == Some("text")
                    && block
                        .get("text")
                        .and_then(Value::as_str)
                        .is_some_and(|text| !text.is_empty())
            }) else {
                return false;
            };
            block.insert("cache_control".into(), marker);
            true
        }
        _ => false,
    }
}
