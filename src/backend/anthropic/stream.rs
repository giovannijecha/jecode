//! Incremental Messages SSE assembly with exact native block replay.

use crate::backend::{Charges, Completion, Failure, FailureKind, ToolCall, Update, Usage};
use crate::json::{self, Value};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Default)]
pub(super) struct Stream {
    pending: Vec<u8>,
    data: String,
    blocks: BTreeMap<usize, Value>,
    open: BTreeSet<usize>,
    tool_json: BTreeMap<usize, String>,
    usage: Option<Usage>,
    stopped: bool,
    stop_reason: Option<String>,
    error: Option<Failure>,
}

impl Stream {
    pub fn feed(
        &mut self,
        bytes: &[u8],
        emit: &mut dyn FnMut(Update) -> Result<(), String>,
    ) -> Result<(), String> {
        self.pending.extend_from_slice(bytes);
        if self.pending.len() > 8 * 1024 * 1024 {
            return Err("Anthropic stream event is too large".into());
        }
        while let Some(end) = self.pending.iter().position(|byte| *byte == b'\n') {
            let line = self.pending.drain(..=end).collect::<Vec<_>>();
            let line = std::str::from_utf8(&line)
                .map_err(|_| "Anthropic returned invalid UTF-8")?
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
        if self.stopped {
            return Err("Anthropic sent an event after message_stop".into());
        }
        let event = json::parse(data)?;
        let kind = event
            .get("type")
            .and_then(Value::as_str)
            .ok_or("Anthropic stream event has no type")?;
        match kind {
            "message_start" => {
                if let Some(usage) = event
                    .get("message")
                    .and_then(|message| message.get("usage"))
                    .and_then(usage)
                {
                    self.update_usage(usage, emit)?;
                }
            }
            "content_block_start" => {
                let index = index(&event)?;
                let block = event
                    .get("content_block")
                    .ok_or("Anthropic content block is missing")?;
                if self.blocks.insert(index, block.clone()).is_some() {
                    return Err("Anthropic repeated a content block index".into());
                }
                self.open.insert(index);
                match block.get("type").and_then(Value::as_str) {
                    Some("thinking" | "redacted_thinking") => emit(Update::Reasoning)?,
                    Some("tool_use") => emit(Update::Working)?,
                    _ => {}
                }
            }
            "content_block_delta" => {
                let index = index(&event)?;
                if !self.open.contains(&index) {
                    return Err("Anthropic delta has no open content block".into());
                }
                let delta = event
                    .get("delta")
                    .ok_or("Anthropic content delta is missing")?;
                let kind = delta
                    .get("type")
                    .and_then(Value::as_str)
                    .ok_or("Anthropic content delta has no type")?;
                let block = self
                    .blocks
                    .get_mut(&index)
                    .ok_or("Anthropic delta has no content block")?;
                match kind {
                    "text_delta" => {
                        let text = delta
                            .get("text")
                            .and_then(Value::as_str)
                            .ok_or("Anthropic text delta is missing")?;
                        append(block, "text", text)?;
                        if !text.is_empty() {
                            emit(Update::TextDelta(text.into()))?;
                        }
                    }
                    "thinking_delta" => {
                        append(
                            block,
                            "thinking",
                            delta
                                .get("thinking")
                                .and_then(Value::as_str)
                                .ok_or("Anthropic thinking delta is missing")?,
                        )?;
                        emit(Update::Reasoning)?;
                    }
                    "signature_delta" => append(
                        block,
                        "signature",
                        delta
                            .get("signature")
                            .and_then(Value::as_str)
                            .ok_or("Anthropic signature delta is missing")?,
                    )?,
                    "input_json_delta" => {
                        self.tool_json.entry(index).or_default().push_str(
                            delta
                                .get("partial_json")
                                .and_then(Value::as_str)
                                .ok_or("Anthropic tool JSON delta is missing")?,
                        );
                        emit(Update::Working)?;
                    }
                    _ => {}
                }
            }
            "content_block_stop" => {
                let index = index(&event)?;
                if !self.open.remove(&index) {
                    return Err("Anthropic stopped an unopened content block".into());
                }
                if let Some(json) = self.tool_json.remove(&index)
                    && !json.is_empty()
                {
                    let input = json::parse(&json)?;
                    let Value::Object(fields) = self
                        .blocks
                        .get_mut(&index)
                        .ok_or("Anthropic tool block is missing")?
                    else {
                        return Err("Anthropic tool block is invalid".into());
                    };
                    fields.insert("input".into(), input);
                }
            }
            "message_delta" => {
                if let Some(reason) = event
                    .get("delta")
                    .and_then(|delta| delta.get("stop_reason"))
                    .and_then(Value::as_str)
                {
                    self.stop_reason = Some(reason.into());
                }
                if let Some(usage) = event.get("usage").and_then(usage) {
                    self.update_usage(usage, emit)?;
                }
            }
            "message_stop" => self.stopped = true,
            "error" => self.error = Some(super::transport::failure(Some(&event), None)),
            "ping" => {}
            _ => {}
        }
        Ok(())
    }

    fn update_usage(
        &mut self,
        usage: Usage,
        emit: &mut dyn FnMut(Update) -> Result<(), String>,
    ) -> Result<(), String> {
        let current = self.usage.get_or_insert_default();
        current.update(usage);
        emit(Update::Usage(*current))
    }

    pub fn finish(self) -> Result<Completion, Failure> {
        if !self.pending.is_empty() || !self.data.is_empty() {
            return Err(Failure::temporary("Anthropic stream ended inside an event"));
        }
        if let Some(error) = self.error {
            return Err(error);
        }
        if !self.open.is_empty() || !self.tool_json.is_empty() {
            return Err(Failure::temporary(
                "Anthropic stream ended inside a content block",
            ));
        }
        if !self.stopped {
            return Err(Failure::temporary(
                "Anthropic stream ended before message_stop",
            ));
        }
        if self.stop_reason.as_deref() == Some("max_tokens") {
            return Err(Failure {
                kind: FailureKind::Length,
                reason: None,
                message: "Anthropic reached max_tokens".into(),
                diagnostic: None,
                retry_after: None,
                charges: None,
            });
        }
        if self.stop_reason.as_deref() == Some("model_context_window_exceeded") {
            return Err(Failure {
                kind: FailureKind::Context,
                reason: None,
                message: "Anthropic reached the model context window".into(),
                diagnostic: None,
                retry_after: None,
                charges: None,
            });
        }
        let blocks: Vec<Value> = self.blocks.into_values().collect();
        let mut text = String::new();
        let mut calls = Vec::new();
        for block in &blocks {
            match block.get("type").and_then(Value::as_str) {
                Some("text") => {
                    if let Some(part) = block.get("text").and_then(Value::as_str) {
                        text.push_str(part);
                    }
                }
                Some("tool_use") => {
                    let field = |name| {
                        block.get(name).and_then(Value::as_str).ok_or_else(|| {
                            Failure::from(format!("Anthropic tool call has no {name}"))
                        })
                    };
                    calls.push(ToolCall {
                        id: field("id")?.into(),
                        name: field("name")?.into(),
                        arguments: block
                            .get("input")
                            .ok_or_else(|| Failure::from("Anthropic tool call has no input"))?
                            .encode(),
                    });
                }
                _ => {}
            }
        }
        if text.is_empty() && calls.is_empty() {
            return Err(Failure {
                kind: FailureKind::Empty,
                reason: None,
                message: "Anthropic completed without text or executable tool calls".into(),
                diagnostic: None,
                retry_after: None,
                charges: None,
            });
        }
        let mut message = Value::object([
            ("role", Value::string("assistant")),
            ("content", Value::string(&text)),
            ("anthropic_blocks", Value::Array(blocks)),
        ]);
        if !calls.is_empty()
            && let Value::Object(fields) = &mut message
        {
            fields.insert(
                "tool_calls".into(),
                Value::Array(calls.iter().map(ToolCall::value).collect()),
            );
        }
        let charges = self.usage.as_ref().and_then(|usage| {
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
            usage: self.usage,
            charges,
            postprocess_error: None,
        })
    }
}

fn index(event: &Value) -> Result<usize, String> {
    event
        .get("index")
        .and_then(Value::as_usize)
        .ok_or("Anthropic content block has no index".into())
}

fn append(block: &mut Value, key: &str, delta: &str) -> Result<(), String> {
    let Value::Object(fields) = block else {
        return Err("Anthropic content block is invalid".into());
    };
    match fields.get_mut(key) {
        Some(Value::String(text)) => text.push_str(delta),
        None => {
            fields.insert(key.into(), Value::string(delta));
        }
        _ => return Err(format!("Anthropic content block {key} is invalid")),
    }
    Ok(())
}

fn usage(value: &Value) -> Option<Usage> {
    let uncached = value.get("input_tokens").and_then(Value::as_usize);
    let cached = value
        .get("cache_read_input_tokens")
        .and_then(Value::as_usize);
    let written = value
        .get("cache_creation_input_tokens")
        .and_then(Value::as_usize);
    let input_tokens = (uncached.is_some() || cached.is_some() || written.is_some()).then(|| {
        uncached
            .unwrap_or(0)
            .saturating_add(cached.unwrap_or(0))
            .saturating_add(written.unwrap_or(0))
    });
    let usage = Usage {
        input_tokens,
        output_tokens: value.get("output_tokens").and_then(Value::as_usize),
        total_tokens: None,
        cached_tokens: cached,
        cache_write_tokens: written,
        reasoning_tokens: value
            .get("output_tokens_details")
            .and_then(|details| details.get("thinking_tokens"))
            .and_then(Value::as_usize),
    };
    (usage != Usage::default()).then_some(usage)
}
