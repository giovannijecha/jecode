//! SSE assembly and terminal Responses validation.

use crate::backend::{Completion, Failure, FailureKind, ToolCall, Update, Usage};
use crate::json::{self, Value};
use std::collections::BTreeMap;

#[derive(Default)]
pub(super) struct Stream {
    pending: Vec<u8>,
    data: String,
    items: BTreeMap<usize, Value>,
    terminal: Option<Result<Value, Failure>>,
    observed_text: String,
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
                if let Some(delta) = event.get("delta").and_then(Value::as_str) {
                    self.observed_text.push_str(delta);
                    emit(Update::TextDelta(delta.into()))?;
                }
            }
            "response.reasoning_text.delta" | "response.reasoning_summary_text.delta" => {
                emit(Update::Reasoning)?;
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
                    if let Some(parts) = item.get("content").and_then(Value::as_array) {
                        for part in parts {
                            match part.get("type").and_then(Value::as_str) {
                                Some("output_text") => {
                                    if let Some(value) = part.get("text").and_then(Value::as_str) {
                                        text.push_str(value);
                                    }
                                }
                                Some("refusal") => {
                                    if let Some(value) = part.get("refusal").and_then(Value::as_str)
                                    {
                                        text.push_str(value);
                                    }
                                }
                                _ => {}
                            }
                        }
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
        Ok(Completion {
            message,
            text,
            calls,
            usage,
            charges: None,
            postprocess_error: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn completed_empty_output_keeps_items_already_delivered_by_the_chatgpt_route() {
        let bytes = concat!(
            "data: {\"type\":\"response.output_item.done\",\"output_index\":0,\"item\":{\"type\":\"reasoning\",\"encrypted_content\":\"opaque\"}}\n\n",
            "data: {\"type\":\"response.output_item.done\",\"output_index\":1,\"item\":{\"type\":\"message\",\"role\":\"assistant\",\"phase\":\"commentary\",\"content\":[{\"type\":\"output_text\",\"text\":\"Checking the file\"}]}}\n\n",
            "data: {\"type\":\"response.output_item.done\",\"output_index\":2,\"item\":{\"type\":\"function_call\",\"call_id\":\"read-1\",\"namespace\":\"jecode\",\"name\":\"read\",\"arguments\":\"{\\\"path\\\":\\\"probe.txt\\\"}\"}}\n\n",
            "data: {\"type\":\"response.completed\",\"response\":{\"status\":\"completed\",\"output\":[],\"usage\":{\"input_tokens\":12,\"output_tokens\":5}}}\n\n"
        );
        let mut stream = Stream::default();
        for chunk in bytes.as_bytes().chunks(11) {
            stream.feed(chunk, &mut |_| Ok(())).unwrap();
        }
        let completion = stream.finish().unwrap();
        assert_eq!(completion.text, "Checking the file");
        assert_eq!(completion.calls[0].name, "read");
        assert_eq!(
            completion
                .message
                .get("openai_items")
                .unwrap()
                .as_array()
                .unwrap()
                .len(),
            3
        );
        assert_eq!(completion.usage.unwrap().input_tokens, Some(12));
    }

    #[test]
    fn fragmented_stream_waits_for_completion_and_preserves_native_order() {
        let mut stream = Stream::default();
        let bytes = concat!(
            "data: {\"type\":\"response.output_text.delta\",\"delta\":\"Hi\"}\n\n",
            "data: {\"type\":\"response.output_item.done\",\"output_index\":0,\"item\":{\"type\":\"reasoning\",\"encrypted_content\":\"secret\"}}\n\n",
            "data: {\"type\":\"response.completed\",\"response\":{\"status\":\"completed\",\"output\":[{\"type\":\"reasoning\"},{\"type\":\"message\",\"role\":\"assistant\",\"phase\":\"final\",\"content\":[{\"type\":\"output_text\",\"text\":\"Hi\"}]}],\"usage\":{\"input_tokens\":3,\"output_tokens\":2}}}\n\n"
        );
        let mut seen = Vec::new();
        for chunk in bytes.as_bytes().chunks(7) {
            stream
                .feed(chunk, &mut |update| {
                    if let Update::TextDelta(text) = update {
                        seen.push(text);
                    }
                    Ok(())
                })
                .unwrap();
        }
        let result = stream.finish().unwrap();
        assert_eq!(seen, ["Hi"]);
        assert_eq!(result.text, "Hi");
        let items = result
            .message
            .get("openai_items")
            .unwrap()
            .as_array()
            .unwrap();
        assert_eq!(
            items[0].get("encrypted_content").and_then(Value::as_str),
            Some("secret")
        );
        assert_eq!(result.usage.unwrap().input_tokens, Some(3));
    }

    #[test]
    fn interrupted_or_failed_stream_never_returns_calls() {
        let call = "data: {\"type\":\"response.output_item.done\",\"output_index\":0,\"item\":{\"type\":\"function_call\",\"call_id\":\"c\",\"name\":\"write\",\"arguments\":\"{}\"}}\n\n";
        let mut stream = Stream::default();
        stream.feed(call.as_bytes(), &mut |_| Ok(())).unwrap();
        assert_eq!(stream.finish().err().unwrap().kind, FailureKind::Temporary);
        let mut stream = Stream::default();
        stream.feed(call.as_bytes(), &mut |_| Ok(())).unwrap();
        stream.feed(b"data: {\"type\":\"response.failed\",\"response\":{\"error\":{\"code\":\"subscription_sharing_usage_limit_exceeded\",\"message\":\"limit\"}}}\n\n", &mut |_| Ok(())).unwrap();
        let failure = stream.finish().err().unwrap();
        assert_eq!(failure.kind, FailureKind::Terminal);
        assert!(failure.diagnostic.as_deref().unwrap().contains("\"error\""));
        assert!(
            failure
                .diagnostic
                .as_deref()
                .unwrap()
                .contains("response.failed")
        );
    }

    #[test]
    fn completed_refusal_is_displayed_as_assistant_text() {
        let mut stream = Stream::default();
        let bytes = concat!(
            "data: {\"type\":\"response.refusal.delta\",\"delta\":\"I cannot help\"}\n\n",
            "data: {\"type\":\"response.completed\",\"response\":{\"status\":\"completed\",\"output\":[{\"type\":\"message\",\"role\":\"assistant\",\"content\":[{\"type\":\"refusal\",\"refusal\":\"I cannot help\"}]}]}}\n\n",
        );
        let mut deltas = Vec::new();
        stream
            .feed(bytes.as_bytes(), &mut |update| {
                if let Update::TextDelta(text) = update {
                    deltas.push(text);
                }
                Ok(())
            })
            .unwrap();
        let completion = stream.finish().unwrap();
        assert_eq!(deltas, ["I cannot help"]);
        assert_eq!(completion.text, "I cannot help");
        assert!(completion.calls.is_empty());
    }
}
