use super::{Completion, api_error, completion::parse_completion};
use crate::json::{self, Value};
use std::collections::BTreeMap;

pub enum Update {
    Reasoning,
    Text(String),
    Working,
    Retry {
        attempt: usize,
        delay: std::time::Duration,
        error: String,
    },
    RetryFinished,
}

#[derive(Default)]
pub struct Stream {
    buffer: Vec<u8>,
    data: String,
    text: String,
    calls: BTreeMap<usize, Value>,
    reasoning: Vec<Value>,
    reasoning_text: String,
    finish: Option<String>,
    done: bool,
    saw_data: bool,
    pub(super) failure: Option<super::Failure>,
    usage: Option<Value>,
}

impl Stream {
    pub fn feed(
        &mut self,
        bytes: &[u8],
        emit: &mut impl FnMut(Update) -> Result<(), String>,
    ) -> Result<(), String> {
        self.buffer.extend_from_slice(bytes);
        while let Some(end) = self.buffer.iter().position(|byte| *byte == b'\n') {
            let line = String::from_utf8(self.buffer.drain(..=end).collect())
                .map_err(|_| "OpenRouter returned invalid UTF-8")?;
            let line = line.trim_end_matches(['\r', '\n']);
            if line.is_empty() {
                if !self.data.is_empty() {
                    let data = std::mem::take(&mut self.data);
                    self.event(data.trim_end(), emit)?;
                }
            } else if let Some(data) = line.strip_prefix("data:") {
                self.saw_data = true;
                self.data.push_str(data.strip_prefix(' ').unwrap_or(data));
                self.data.push('\n');
            }
            // SSE comments, event/id fields and curl's final status are not deltas.
        }
        Ok(())
    }

    pub fn finish(self, body: &str) -> Result<Completion, String> {
        if !self.saw_data {
            return parse_completion(json::parse(body)?);
        }
        if !self.done {
            return Err(
                "OpenRouter stream ended before [DONE]; no tools from this response were executed"
                    .into(),
            );
        }
        if self
            .calls
            .keys()
            .enumerate()
            .any(|(position, index)| position != *index)
        {
            return Err("OpenRouter stream has missing tool call indexes; no tools from this response were executed".into());
        }
        let mut message = Value::object([
            ("role", Value::string("assistant")),
            ("content", Value::string(self.text)),
        ]);
        let Value::Object(fields) = &mut message else {
            unreachable!()
        };
        if !self.calls.is_empty() {
            fields.insert(
                "tool_calls".into(),
                Value::Array(self.calls.into_values().collect()),
            );
        }
        if !self.reasoning.is_empty() {
            fields.insert("reasoning_details".into(), Value::Array(self.reasoning));
        }
        if !self.reasoning_text.is_empty() {
            fields.insert("reasoning".into(), Value::string(self.reasoning_text));
        }
        let mut response = Value::object([(
            "choices",
            Value::Array(vec![Value::object([
                (
                    "finish_reason",
                    self.finish.map_or(Value::Null, Value::string),
                ),
                ("message", message),
            ])]),
        )]);
        if let Some(usage) = self.usage
            && let Value::Object(fields) = &mut response
        {
            fields.insert("usage".into(), usage);
        }
        parse_completion(response)
    }

    fn event(
        &mut self,
        data: &str,
        emit: &mut impl FnMut(Update) -> Result<(), String>,
    ) -> Result<(), String> {
        if data == "[DONE]" {
            self.done = true;
            return Ok(());
        }
        if self.done {
            return Err("OpenRouter sent data after [DONE]".into());
        }
        let value = json::parse(data)?;
        if let Some(error) = api_error(&value) {
            self.failure = Some(super::Failure::api(&value, 200));
            return Err(format!("OpenRouter stream error: {error}"));
        }
        if let Some(usage) = value.get("usage") {
            self.usage = Some(usage.clone());
        }
        let choices = value
            .get("choices")
            .and_then(Value::as_array)
            .ok_or("OpenRouter stream has invalid choices")?;
        if choices.is_empty() {
            return Ok(());
        } // Final usage-only chunk.
        if choices.len() != 1 {
            return Err("OpenRouter stream returned multiple choices".into());
        }
        let choice = &choices[0];
        if choice
            .get("index")
            .and_then(Value::as_usize)
            .is_some_and(|index| index != 0)
        {
            return Err("OpenRouter stream returned an unexpected choice index".into());
        }
        if let Some(error) = api_error(choice) {
            self.failure = Some(super::Failure::api(choice, 200));
            return Err(format!("OpenRouter stream error: {error}"));
        }
        if let Some(reason) = choice.get("finish_reason").and_then(Value::as_str) {
            if self
                .finish
                .as_ref()
                .is_some_and(|previous| previous != reason)
            {
                return Err("OpenRouter stream changed its finish reason".into());
            }
            self.finish = Some(reason.into());
        }
        let Some(delta) = choice.get("delta") else {
            return Ok(());
        };
        if delta
            .get("role")
            .and_then(Value::as_str)
            .is_some_and(|role| role != "assistant")
        {
            return Err("OpenRouter stream returned an invalid assistant role".into());
        }
        if let Some(content) = delta.get("content") {
            match content {
                Value::String(text) if !text.is_empty() => {
                    self.text.push_str(text);
                    emit(Update::Text(self.text.clone()))?;
                }
                Value::Null | Value::String(_) => {}
                _ => return Err("OpenRouter stream returned unsupported content".into()),
            }
        }
        if let Some(details) = delta.get("reasoning_details") {
            match details {
                Value::Array(details) if !details.is_empty() => {
                    self.reasoning.extend(details.iter().cloned());
                    emit(Update::Reasoning)?;
                }
                Value::Null | Value::Array(_) => {}
                _ => return Err("OpenRouter stream returned invalid reasoning details".into()),
            }
        }
        if let Some(text) = delta
            .get("reasoning")
            .and_then(Value::as_str)
            .or_else(|| delta.get("reasoning_content").and_then(Value::as_str))
            && !text.is_empty()
        {
            self.reasoning_text.push_str(text);
            emit(Update::Reasoning)?;
        }
        if let Some(calls) = delta
            .get("tool_calls")
            .filter(|value| !matches!(value, Value::Null))
        {
            let calls = calls
                .as_array()
                .ok_or("OpenRouter stream returned invalid tool calls")?;
            for call in calls {
                self.call(call)?;
            }
            if !calls.is_empty() {
                emit(Update::Working)?;
            }
        }
        Ok(())
    }

    fn call(&mut self, delta: &Value) -> Result<(), String> {
        let index = delta
            .get("index")
            .and_then(Value::as_usize)
            .ok_or("Stream tool call has no index")?;
        let call = self.calls.entry(index).or_insert_with(|| {
            Value::object([
                ("id", Value::string("")),
                ("type", Value::string("function")),
                (
                    "function",
                    Value::object([
                        ("name", Value::string("")),
                        ("arguments", Value::string("")),
                    ]),
                ),
            ])
        });
        let Value::Object(fields) = call else {
            unreachable!()
        };
        for key in ["id", "type"] {
            if let Some(value) = delta.get(key).filter(|value| !matches!(value, Value::Null)) {
                let text = value
                    .as_str()
                    .ok_or("Stream tool call has an invalid field")?;
                if key == "type" && text != "function" {
                    return Err("Unsupported streamed tool type".into());
                }
                if key == "id" {
                    let previous = fields.get(key).and_then(Value::as_str).unwrap_or("");
                    if !previous.is_empty() && previous != text {
                        return Err("Stream tool call changed its ID".into());
                    }
                }
                fields.insert(key.into(), Value::string(text));
            }
        }
        if let Some(function) = delta.get("function") {
            let Some(Value::Object(fields)) = fields.get_mut("function") else {
                unreachable!()
            };
            for key in ["name", "arguments"] {
                if let Some(value) = function
                    .get(key)
                    .filter(|value| !matches!(value, Value::Null))
                {
                    let text = value
                        .as_str()
                        .ok_or("Stream tool call has an invalid function field")?;
                    let previous = fields.get(key).and_then(Value::as_str).unwrap_or("");
                    fields.insert(key.into(), Value::string(format!("{previous}{text}")));
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
