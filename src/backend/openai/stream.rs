//! SSE assembly and terminal Responses validation.

use crate::backend::{Charges, Completion, Failure, FailureKind, ToolCall, Update, Usage};
use crate::json::{self, Value};
use std::collections::BTreeMap;

#[derive(Default)]
pub(super) struct Stream {
    pending: Vec<u8>,
    data: String,
    items: BTreeMap<usize, Value>,
    terminal: Option<Result<Value, Failure>>,
    has_text: bool,
    text_item: Option<usize>,
    message_boundary: bool,
}

impl Stream {
    pub fn feed(
        &mut self,
        bytes: &[u8],
        emit: &mut dyn FnMut(Update) -> Result<(), String>,
    ) -> Result<(), String> {
        self.pending.extend_from_slice(bytes);
        while let Some(end) = self.pending.iter().position(|byte| *byte == b'\n') {
            let line = self.pending.drain(..=end).collect::<Vec<_>>();
            let line = std::str::from_utf8(&line)
                .map_err(|_| "OpenAI returned invalid UTF-8")?
                .trim_end_matches(['\r', '\n']);
            if line.is_empty() {
                if !self.data.is_empty() {
                    let data = std::mem::take(&mut self.data);
                    self.event(data.trim_end(), emit)?;
                }
            } else if let Some(value) = line.strip_prefix("data:") {
                self.data.push_str(value.strip_prefix(' ').unwrap_or(value));
                self.data.push('\n');
            }
        }
        Ok(())
    }

    fn event(
        &mut self,
        data: &str,
        emit: &mut dyn FnMut(Update) -> Result<(), String>,
    ) -> Result<(), String> {
        if data == "[DONE]" {
            return Ok(());
        }
        if self.terminal.is_some() {
            return Err("OpenAI sent an event after the terminal response".into());
        }
        let event = json::parse(data)?;
        match event
            .get("type")
            .and_then(Value::as_str)
            .ok_or("OpenAI stream event has no type")?
        {
            "response.output_text.delta" | "response.refusal.delta" => {
                if let Some(delta) = event.get("delta").and_then(Value::as_str)
                    && !delta.is_empty()
                {
                    let index = event.get("output_index").and_then(Value::as_usize);
                    if self.has_text
                        && (self.message_boundary || (index.is_some() && index != self.text_item))
                    {
                        emit(Update::TextDelta("\n\n".into()))?;
                    }
                    self.has_text = true;
                    self.text_item = index.or(self.text_item);
                    self.message_boundary = false;
                    emit(Update::TextDelta(delta.into()))?;
                }
            }
            "response.reasoning_text.delta" | "response.reasoning_summary_text.delta" => {
                emit(Update::Reasoning)?;
            }
            // Reasoning without a requested summary streams no deltas; the
            // opened item is the only sign that the model is working.
            "response.output_item.added" => {
                match event
                    .get("item")
                    .and_then(|item| item.get("type"))
                    .and_then(Value::as_str)
                {
                    Some("reasoning") => emit(Update::Reasoning)?,
                    Some("function_call" | "custom_tool_call") => emit(Update::Working)?,
                    Some("message") => self.message_boundary = self.has_text,
                    _ => {}
                }
            }
            "response.output_item.done" => {
                let index = event
                    .get("output_index")
                    .and_then(Value::as_usize)
                    .ok_or("OpenAI completed item has no output index")?;
                let item = event
                    .get("item")
                    .ok_or("OpenAI completed item has no item")?;
                self.items.insert(index, item.clone());
            }
            "response.completed" => {
                let response = event
                    .get("response")
                    .ok_or("OpenAI completed event has no response")?;
                if response.get("status").and_then(Value::as_str) != Some("completed") {
                    return Err("OpenAI completed event has an invalid status".into());
                }
                self.terminal = Some(Ok(response.clone()));
            }
            "response.failed" | "response.incomplete" => {
                let response = event.get("response");
                let mut failure = super::transport::failure(response, None);
                failure.diagnostic = Some(event.encode());
                if event.get("type").and_then(Value::as_str) == Some("response.incomplete") {
                    let reason = response
                        .and_then(|response| response.get("incomplete_details"))
                        .and_then(|details| details.get("reason"))
                        .and_then(Value::as_str);
                    if let Some(reason) = reason {
                        failure
                            .message
                            .push_str(&format!("; incomplete reason: {reason}"));
                        if reason == "max_output_tokens" {
                            failure.kind = FailureKind::Length;
                        }
                    }
                }
                self.terminal = Some(Err(failure));
            }
            "error" => {
                self.terminal = Some(Err(super::transport::failure(Some(&event), None)));
            }
            _ => {}
        }
        Ok(())
    }

    pub fn finish(self) -> Result<Completion, Failure> {
        if !self.pending.is_empty() || !self.data.is_empty() {
            return Err(Failure::temporary("OpenAI stream ended inside an event"));
        }
        let response = self
            .terminal
            .ok_or_else(|| Failure::temporary("OpenAI stream ended before response.completed"))??;
        let output = response
            .get("output")
            .and_then(Value::as_array)
            .ok_or_else(|| Failure::from("OpenAI completed response has no output"))?;
        // The ChatGPT route can end with output: [] after sending every item
        // through output_item.done. Keep those completed items independently
        // of whether the terminal response repeats them.
        let mut items = self.items;
        for (index, item) in output.iter().enumerate() {
            items.entry(index).or_insert_with(|| item.clone());
        }
        let items: Vec<Value> = items.into_values().collect();
        let mut text = String::new();
        let mut calls = Vec::new();
        for item in &items {
            match item.get("type").and_then(Value::as_str) {
                Some("message") => {
                    if item.get("role").and_then(Value::as_str) != Some("assistant") {
                        continue;
                    }
                    let mut message_text = String::new();
                    if let Some(parts) = item.get("content").and_then(Value::as_array) {
                        for part in parts {
                            match part.get("type").and_then(Value::as_str) {
                                Some("output_text") => {
                                    if let Some(value) = part.get("text").and_then(Value::as_str) {
                                        message_text.push_str(value);
                                    }
                                }
                                Some("refusal") => {
                                    if let Some(value) = part.get("refusal").and_then(Value::as_str)
                                    {
                                        message_text.push_str(value);
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                    if !message_text.is_empty() {
                        if !text.is_empty() {
                            text.push_str("\n\n");
                        }
                        text.push_str(&message_text);
                    }
                }
                Some("function_call") => {
                    let field = |name| {
                        item.get(name).and_then(Value::as_str).ok_or_else(|| {
                            Failure::from(format!("OpenAI function call has no {name}"))
                        })
                    };
                    let name = match item.get("namespace").and_then(Value::as_str) {
                        Some(namespace) if namespace != "jecode" => {
                            format!("{namespace}.{}", field("name")?)
                        }
                        _ => field("name")?.into(),
                    };
                    calls.push(ToolCall {
                        id: field("call_id")?.into(),
                        name,
                        arguments: field("arguments")?.into(),
                    });
                }
                _ => {}
            }
        }
        if text.is_empty() && calls.is_empty() {
            return Err(Failure {
                kind: FailureKind::Empty,
                reason: None,
                message: "OpenAI completed without text or executable tool calls".into(),
                diagnostic: None,
                retry_after: None,
                charges: None,
            });
        }
        let mut message = Value::object([
            ("role", Value::string("assistant")),
            ("content", Value::string(&text)),
            ("openai_items", Value::Array(items)),
        ]);
        if !calls.is_empty() {
            let Value::Object(fields) = &mut message else {
                unreachable!()
            };
            fields.insert(
                "tool_calls".into(),
                Value::Array(calls.iter().map(ToolCall::value).collect()),
            );
        }
        let usage = response.get("usage").map(|usage| Usage {
            input_tokens: usage.get("input_tokens").and_then(Value::as_usize),
            output_tokens: usage.get("output_tokens").and_then(Value::as_usize),
            cached_tokens: usage
                .get("input_tokens_details")
                .and_then(|details| details.get("cached_tokens"))
                .and_then(Value::as_usize),
            cache_write_tokens: usage
                .get("input_tokens_details")
                .and_then(|details| details.get("cache_write_tokens"))
                .and_then(Value::as_usize),
        });
        // The plan reports no prices; cache reads still show the cache at work.
        let charges = usage.as_ref().and_then(|usage| {
            let cached = usage.cached_tokens.unwrap_or(0);
            let cache_written = usage.cache_write_tokens.unwrap_or(0);
            (cached > 0 || cache_written > 0).then(|| Charges {
                requests: 1,
                cached,
                cache_written,
                ..Charges::default()
            })
        });
        Ok(Completion {
            message,
            text,
            calls,
            usage,
            charges,
            postprocess_error: None,
        })
    }
}

#[cfg(test)]
mod tests;
