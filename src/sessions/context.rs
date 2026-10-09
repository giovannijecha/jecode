use super::{
    Document, Pending,
    document::{integer, string},
};
use crate::json::{self, Value};
use std::collections::BTreeSet;

pub(super) fn validate(document: &Document) -> Result<(), String> {
    validate_from(document, &mut Validation::default())
}

#[derive(Clone, Default)]
pub(super) struct Validation {
    messages: usize,
    events: usize,
    outstanding: BTreeSet<String>,
}

pub(super) fn validate_from(document: &Document, validated: &mut Validation) -> Result<(), String> {
    let messages = &document.messages;
    document.context.validate(messages)?;
    if messages
        .first()
        .and_then(|message| message.get("role"))
        .and_then(Value::as_str)
        != Some("system")
        || document.compatible_from > messages.len()
    {
        return Err("Invalid session context or model compatibility boundary".into());
    }
    if messages.len() < validated.messages || document.events.len() < validated.events {
        return Err("Saved session history cannot shrink".into());
    }
    let mut next = validated.clone();
    let outstanding = &mut next.outstanding;
    for (index, message) in messages.iter().enumerate().skip(validated.messages) {
        let role = string(message, "role")?;
        if !outstanding.is_empty() && role != "tool" {
            return Err("Session has tool calls without adjacent results".into());
        }
        match role {
            "system" if index == 0 => {
                string(message, "content")?;
            }
            "user" => {
                string(message, "content")?;
            }
            "assistant" => {
                if !matches!(
                    message.get("content"),
                    None | Some(Value::Null | Value::String(_))
                ) {
                    return Err("Unsupported saved assistant content".into());
                }
                if let Some(Value::Array(calls)) = message.get("tool_calls") {
                    for call in calls {
                        let id = string(call, "id")?;
                        if id.is_empty()
                            || !outstanding.insert(id.to_string())
                            || string(call, "type")? != "function"
                        {
                            return Err("Invalid saved tool call".into());
                        }
                        let function = call.get("function").ok_or("Missing saved tool function")?;
                        string(function, "name")?;
                        string(function, "arguments")?;
                    }
                } else if !matches!(message.get("tool_calls"), None | Some(Value::Null)) {
                    return Err("Invalid saved tool calls".into());
                }
            }
            "tool" if outstanding.remove(string(message, "tool_call_id")?) => {
                let result = json::parse(string(message, "content")?)?;
                if !matches!(result, Value::Object(_)) {
                    return Err("Invalid saved tool result".into());
                }
            }
            _ => return Err("Invalid message role or tool result in session".into()),
        }
    }
    if !outstanding.is_empty() && !document.pending.active {
        return Err("Finished session has unresolved tool calls".into());
    }
    if let Some(id) = &document.pending.tool
        && (!document.pending.active || !outstanding.contains(id))
    {
        return Err("Pending tool does not match the saved context".into());
    }
    for event in document.events.iter().skip(validated.events) {
        if let Some(kind) = event.get("kind")
            && !matches!(kind.as_str(), Some("notice" | "warning" | "error"))
        {
            return Err("Invalid saved event kind".into());
        }
        if integer(event, "after_message")? > messages.len() as u64 {
            return Err("Saved event is outside the conversation".into());
        }
        match string(event, "type")? {
            "local_command" => {
                string(event, "command")?;
                string(event, "result")?;
                if let Some(details) = event.get("details") {
                    for row in details.as_array().ok_or("Invalid command details")? {
                        let pair = row.as_array().ok_or("Invalid command detail")?;
                        if pair.len() != 2 || pair.iter().any(|value| value.as_str().is_none()) {
                            return Err("Invalid command detail".into());
                        }
                    }
                }
            }
            "turn_error" | "request_recovery" => {
                string(event, "error")?;
                string(event, "partial_text")?;
            }
            "temporary_cleanup" => {
                string(event, "result")?;
            }
            _ => return Err("Unsupported saved conversation event".into()),
        }
    }
    next.messages = messages.len();
    next.events = document.events.len();
    *validated = next;
    Ok(())
}

impl Document {
    pub(super) fn recover(&mut self) -> bool {
        let interrupted = self.pending.active;
        if interrupted {
            let mut calls = Vec::new();
            let mut answered = BTreeSet::new();
            for message in self.messages.iter().rev() {
                match message.get("role").and_then(Value::as_str) {
                    Some("tool") => {
                        if let Some(id) = message.get("tool_call_id").and_then(Value::as_str) {
                            answered.insert(id.to_string());
                        }
                    }
                    Some("assistant") => {
                        if let Some(Value::Array(values)) = message.get("tool_calls") {
                            calls = values.clone();
                        }
                        break;
                    }
                    _ => break,
                }
            }
            for call in calls {
                let id = call.get("id").and_then(Value::as_str).unwrap();
                if answered.contains(id) {
                    continue;
                }
                let unknown = self.pending.tool.as_deref() == Some(id);
                let result = Value::object([
                    (
                        "error",
                        Value::string(if unknown {
                            "Tool outcome is unknown: Jecode closed after execution started. Check the workspace before deciding whether to run it again."
                        } else {
                            "Tool was not executed: Jecode closed before execution started."
                        }),
                    ),
                    (
                        "outcome",
                        Value::string(if unknown { "unknown" } else { "not_executed" }),
                    ),
                ]);
                self.messages.push(Value::object([
                    ("role", Value::string("tool")),
                    ("tool_call_id", Value::string(id)),
                    ("content", Value::string(result.encode())),
                ]));
            }
            self.events.push(Value::object([
                ("type", Value::string("turn_error")),
                ("kind", Value::string("warning")),
                ("error", Value::string("Recovered an interrupted turn. Partial output is incomplete; send a new request to continue.")),
                ("partial_text", Value::string(&self.pending.partial)),
                ("after_message", Value::number(self.messages.len())),
            ]));
            self.pending = Pending::default();
        }
        self.input.recover();
        interrupted
    }
}
