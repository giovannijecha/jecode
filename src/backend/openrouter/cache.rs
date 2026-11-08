//! Explicit cache boundaries for providers that accept chat content markers.

use crate::backend::CacheRetention;
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

/// A breakpoint marker; one-hour retention adds the TTL Claude accepts.
pub(super) fn marker(retention: CacheRetention) -> Value {
    match retention {
        CacheRetention::Short => Value::object([("type", Value::string("ephemeral"))]),
        CacheRetention::Long => Value::object([
            ("type", Value::string("ephemeral")),
            ("ttl", Value::string("1h")),
        ]),
    }
}

pub(super) fn mark_recent(messages: &mut [Value], marker: &Value) {
    let mut marked = 0;
    // The end of the native records keeps its slot: that prefix outlives compactions.
    let anchor = messages
        .iter()
        .rposition(|message| message.get(crate::context::digest::ANCHOR).is_some());
    if let Some(index) = anchor
        && mark(&mut messages[index], marker)
    {
        marked += 1;
    }
    let mut inside_tool_batch = false;
    for (index, message) in messages.iter_mut().enumerate().rev() {
        let tool = message.get("role").and_then(Value::as_str) == Some("tool");
        if tool && inside_tool_batch {
            continue;
        }
        inside_tool_batch = tool;
        if Some(index) != anchor && mark(message, marker) {
            marked += 1;
            if marked == 4 {
                break;
            }
        }
    }
}

fn mark(message: &mut Value, marker: &Value) -> bool {
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
    match content {
        Value::String(text) if !text.is_empty() => {
            let text = text.clone();
            *content = Value::Array(vec![Value::object([
                ("type", Value::string("text")),
                ("text", Value::string(text)),
                ("cache_control", marker.clone()),
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
            block.insert("cache_control".into(), marker.clone());
            true
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn message(text: &str) -> Value {
        Value::object([
            ("role", Value::string("user")),
            ("content", Value::string(text)),
        ])
    }

    fn marked(message: &Value) -> bool {
        message.encode().contains("cache_control")
    }

    #[test]
    fn the_end_of_the_records_keeps_a_breakpoint_within_the_limit_of_four() {
        let mut messages = (0..8)
            .map(|index| message(&format!("message {index}")))
            .collect::<Vec<_>>();
        if let Value::Object(fields) = &mut messages[1] {
            fields.insert(crate::context::digest::ANCHOR.into(), Value::Bool(true));
        }
        mark_recent(&mut messages, &marker(CacheRetention::Short));
        assert!(marked(&messages[1]), "the anchor outlives recent messages");
        assert_eq!(messages.iter().filter(|message| marked(message)).count(), 4);
        assert!(messages[5..].iter().all(marked), "the latest three follow");
    }
}
