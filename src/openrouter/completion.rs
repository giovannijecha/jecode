use super::{Completion, ToolCall, api_error};
use crate::json::Value;
use std::collections::BTreeSet;

pub(super) fn parse_completion(value: Value) -> Result<Completion, String> {
    if let Some(error) = api_error(&value) {
        return Err(format!("OpenRouter error: {error}"));
    }
    let choice = value
        .get("choices")
        .and_then(Value::as_array)
        .and_then(|choices| choices.first())
        .ok_or("OpenRouter returned no completion choices")?;
    if let Some(error) = api_error(choice) {
        return Err(format!("OpenRouter error: {error}"));
    }
    let finish = choice
        .get("finish_reason")
        .and_then(Value::as_str)
        .ok_or("OpenRouter returned no finish reason")?;
    if finish != "stop" && finish != "tool_calls" {
        return Err(format!(
            "OpenRouter returned an incomplete response (finish_reason: {finish}); no tools from this response were executed"
        ));
    }
    let message = choice
        .get("message")
        .ok_or("OpenRouter returned no assistant message")?;
    if message.get("role").and_then(Value::as_str) != Some("assistant") {
        return Err("OpenRouter returned an invalid assistant role".into());
    }
    let text = match message.get("content") {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Null) | None => String::new(),
        _ => return Err("OpenRouter returned unsupported assistant content".into()),
    };
    let raw_calls = match message.get("tool_calls") {
        Some(Value::Array(calls)) => calls.as_slice(),
        Some(Value::Null) | None => &[],
        _ => return Err("OpenRouter returned invalid tool calls".into()),
    };
    let mut ids = BTreeSet::new();
    let mut calls = Vec::new();
    for call in raw_calls {
        let id = field(call, "id")?;
        if id.is_empty() || !ids.insert(id) {
            return Err("OpenRouter returned empty or duplicate tool call IDs".into());
        }
        if field(call, "type")? != "function" {
            return Err("OpenRouter returned an unsupported tool call type".into());
        }
        let function = call
            .get("function")
            .ok_or("OpenRouter returned a tool call without a function")?;
        calls.push(ToolCall {
            id: id.into(),
            name: field(function, "name")?.into(),
            arguments: field(function, "arguments")?.into(),
        });
    }
    if calls.is_empty() && (finish == "tool_calls" || text.trim().is_empty()) {
        return Err("OpenRouter returned neither text nor executable tool calls".into());
    }
    // Keep the full assistant message, including ordered reasoning_details, for the next request.
    Ok(Completion {
        message: message.clone(),
        text,
        calls,
        usage: value.get("usage").cloned(),
    })
}

fn field<'a>(value: &'a Value, name: &str) -> Result<&'a str, String> {
    value
        .get(name)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("OpenRouter tool call has an invalid {name}"))
}
