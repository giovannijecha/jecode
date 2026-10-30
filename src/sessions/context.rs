use super::{
    Document, Pending,
    document::{integer, string},
};
use crate::{
    conversation::ToolCall,
    json::{self, Value},
};
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
    if document.model.is_none()
        && (document.pending.active
            || messages
                .iter()
                .any(|message| message.get("role").and_then(Value::as_str) != Some("system")))
    {
        return Err("A draft without a selected model cannot contain an execution turn".into());
    }
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
                        let call = ToolCall::parse(call)?;
                        if !outstanding.insert(call.id) {
                            return Err("Invalid saved tool call".into());
                        }
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
    if let Some(draft) = &document.input.question
        && !document.events.iter().any(|event| {
            event.get("type").and_then(Value::as_str) == Some("user_question")
                && event.get("tool_call_id").and_then(Value::as_str) == Some(&draft.id)
        })
    {
        return Err("Question draft does not match the saved question".into());
    }
    if let Some(id) = &document.pending.tool
        && (!document.pending.active || !outstanding.contains(id))
    {
        return Err("Pending tool does not match the saved context".into());
    }
    if !document.pending.reads.is_empty()
        && (!document.pending.active
            || document.pending.tool.is_some()
            || document
                .pending
                .reads
                .iter()
                .any(|id| !outstanding.contains(id))
            || document.pending.reads.iter().collect::<BTreeSet<_>>().len()
                != document.pending.reads.len())
    {
        return Err("Pending reads do not match the saved context".into());
    }
    if !document.pending.batch.is_empty()
        && (!document.pending.active
            || document.pending.tool.is_some()
            || (!document.pending.reads.is_empty()
                && document.pending.reads != document.pending.batch)
            || document
                .pending
                .batch
                .iter()
                .any(|id| !outstanding.contains(id))
            || document.pending.batch.iter().collect::<BTreeSet<_>>().len()
                != document.pending.batch.len())
    {
        return Err("Pending batch does not match the saved context".into());
    }
    let batch: BTreeSet<_> = document.pending.batch.iter().map(String::as_str).collect();
    let mut completed = BTreeSet::new();
    for receipt in &document.pending.completed {
        validate_batch_receipt(receipt)?;
        let id = string(receipt, "id")?;
        if !batch.contains(id) || !completed.insert(id) {
            return Err("Completed batch call does not match the pending batch".into());
        }
    }
    for (event_index, event) in document.events.iter().enumerate().skip(validated.events) {
        if event.get("type").and_then(Value::as_str) != Some("collaboration")
            && let Some(kind) = event.get("kind")
            && !matches!(kind.as_str(), Some("notice" | "warning" | "error"))
        {
            return Err("Invalid saved event kind".into());
        }
        if integer(event, "after_message")? > messages.len() as u64 {
            return Err("Saved event is outside the conversation".into());
        }
        match string(event, "type")? {
            "user_question" => {
                let id = string(event, "tool_call_id")?;
                crate::ask::Request::from_event(event)?;
                if !messages.iter().any(|message| {
                    message
                        .get("tool_calls")
                        .and_then(Value::as_array)
                        .is_some_and(|calls| {
                            calls.iter().any(|call| {
                                ToolCall::parse(call)
                                    .is_ok_and(|call| call.id == id && call.name == "ask_user")
                            })
                        })
                }) {
                    return Err("Saved question has no matching tool call".into());
                }
            }
            "user_answer" => {
                crate::ask::validate_answer(event, &document.events[..event_index])?;
            }
            "collaboration" => {
                let kind = string(event, "kind")?;
                if !matches!(
                    kind,
                    "created"
                        | "sent"
                        | "received"
                        | "returned"
                        | "stopped"
                        | "reviewed"
                        | "closed"
                        | "unknown"
                ) {
                    return Err("Invalid collaborator receipt kind".into());
                }
                integer(event, "sequence")?;
                event
                    .get("records")
                    .and_then(Value::as_array)
                    .ok_or("Invalid collaborator records")?;
            }
            "turn_start" => {
                let index = integer(event, "after_message")? as usize;
                if messages
                    .get(index)
                    .and_then(|message| message.get("role"))
                    .and_then(Value::as_str)
                    != Some("user")
                {
                    return Err("Turn start has no user request".into());
                }
            }
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
            "turn_returned" => {
                let from = integer(event, "from_message")? as usize;
                let end = integer(event, "after_message")? as usize;
                if from == 0
                    || from >= end
                    || end > messages.len()
                    || messages[from..end]
                        .iter()
                        .any(|message| message.get("role").and_then(Value::as_str) != Some("user"))
                {
                    return Err("Returned turn contains more than unanswered input".into());
                }
            }
            "temporary_cleanup" => {
                string(event, "result")?;
            }

            "tool_change" => {
                let index = integer(event, "after_message")? as usize;
                let Some(message) = index.checked_sub(1).and_then(|index| messages.get(index))
                else {
                    return Err("File change has no tool result".into());
                };
                if string(message, "role")? != "tool"
                    || string(message, "tool_call_id")? != string(event, "tool_call_id")?
                {
                    return Err("File change does not match its tool result".into());
                }
                crate::file_change::Change::from_value(
                    event.get("change").ok_or("Missing file change")?,
                )?;
            }
            // Retired event types stay in old journals; nothing reads them.
            _ => {}
        }
    }
    next.messages = messages.len();
    next.events = document.events.len();
    *validated = next;
    Ok(())
}

pub(super) fn validate_batch_receipt(receipt: &Value) -> Result<(), String> {
    if string(receipt, "id")?.is_empty()
        || !matches!(receipt.get("result"), Some(Value::Object(_)))
        || !matches!(receipt.get("timing"), Some(Value::Object(_) | Value::Null))
    {
        return Err("Invalid completed batch call".into());
    }
    match receipt.get("change") {
        Some(Value::Object(_)) => {
            crate::file_change::Change::from_value(receipt.get("change").unwrap())?;
        }
        Some(Value::Null) => {}
        _ => return Err("Invalid completed batch change".into()),
    }
    Ok(())
}

impl Document {
    pub(super) fn recover(&mut self) -> bool {
        let interrupted = self.pending.active;
        if interrupted {
            let question =
                crate::ask::recovery(&self.messages, &self.events).map(|(request, _)| request.id);
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
                if question.as_deref() == Some(id) {
                    continue;
                }
                if let Some(receipt) = self
                    .pending
                    .completed
                    .iter()
                    .find(|receipt| receipt.get("id").and_then(Value::as_str) == Some(id))
                {
                    let result = receipt.get("result").unwrap();
                    self.messages.push(Value::object([
                        ("role", Value::string("tool")),
                        ("tool_call_id", Value::string(id)),
                        ("content", Value::string(result.encode())),
                    ]));
                    let after_message = Value::number(self.messages.len());
                    if let Some(Value::Object(timing)) = receipt.get("timing") {
                        let mut timing = timing.clone();
                        timing.insert("type".into(), Value::string("tool_timing"));
                        timing.insert("tool_call_id".into(), Value::string(id));
                        timing.insert(
                            "name".into(),
                            Value::string(ToolCall::parse(&call).unwrap().name),
                        );
                        timing.insert("after_message".into(), after_message.clone());
                        self.events.push(Value::Object(timing));
                    }
                    if let Some(Value::Object(change)) = receipt.get("change") {
                        self.events.push(Value::object([
                            ("type", Value::string("tool_change")),
                            ("after_message", after_message),
                            ("tool_call_id", Value::string(id)),
                            ("change", Value::Object(change.clone())),
                        ]));
                    }
                    continue;
                }
                // A call still waiting for a permission decision never ran.
                let undecided = crate::permissions::pending_request(&self.events, id);
                let unknown = !undecided
                    && (self.pending.tool.as_deref() == Some(id)
                        || self.pending.reads.iter().any(|read| read == id)
                        || self.pending.batch.iter().any(|started| started == id));
                let result = if undecided {
                    Value::object([
                        (
                            "error",
                            Value::string(
                                "Tool was not executed: Jecode quit before a permission decision.",
                            ),
                        ),
                        ("outcome", Value::string("not_executed")),
                    ])
                } else {
                    Value::object([
                        (
                            "error",
                            Value::string(if unknown {
                                "Tool outcome is unknown: Jecode closed after execution may have started. Check the workspace before deciding whether to run it again."
                            } else {
                                "Tool was not executed: Jecode closed before execution started."
                            }),
                        ),
                        (
                            "outcome",
                            Value::string(if unknown { "unknown" } else { "not_executed" }),
                        ),
                    ])
                };
                self.messages.push(Value::object([
                    ("role", Value::string("tool")),
                    ("tool_call_id", Value::string(id)),
                    ("content", Value::string(result.encode())),
                ]));
            }
            if question.is_none() {
                self.events.push(Value::object([
                ("type", Value::string("turn_error")),
                ("kind", Value::string("warning")),
                ("error", Value::string("Recovered an interrupted turn. Partial output is incomplete; send a new request to continue.")),
                ("partial_text", Value::string(&self.pending.partial)),
                ("after_message", Value::number(self.messages.len())),
            ]));
            }
            self.pending = question.map_or_else(Pending::default, |id| Pending {
                active: true,
                tool: Some(id),
                ..Pending::default()
            });
        }
        self.input.recover();
        interrupted
    }
}
