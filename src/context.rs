use crate::{json::Value, openrouter::Limits};

pub(crate) mod source;
mod view;

pub(crate) use view::excerpt;

/// Introduces a compaction summary, which the same model wrote in an earlier context window.
const SUMMARY_PREFIX: &str = "Another language model started this task and wrote the summary below. The work it did with tools is still in place: files, saved outputs and the original history. Build on it and avoid repeating finished work.";

/// A view of durable history. Compaction never deletes the source messages.
#[derive(Clone, Debug)]
pub struct Context {
    pub from: usize,
    pub summary: String,
    pub ceiling: Option<usize>,
    pub input_tokens: Option<usize>,
    pub measured_end: usize,
    pub calibration: Option<(usize, usize)>,
    pub reply_tokens: usize,
    /// Output allowance learned from summary truncation, independent of live prompt usage.
    pub summary_output_tokens: usize,
    pub preview_until: usize,
    pub preview_limit: usize,
    /// Byte budget for verbatim recent user requests after compaction.
    pub request_limit: usize,
}
impl Default for Context {
    fn default() -> Self {
        Self {
            from: 1,
            summary: String::new(),
            ceiling: None,
            input_tokens: None,
            measured_end: 0,
            calibration: None,
            reply_tokens: 0,
            summary_output_tokens: 0,
            preview_until: 0,
            preview_limit: 0,
            request_limit: 0,
        }
    }
}
impl Context {
    pub fn value(&self) -> Value {
        Value::object([
            ("from", Value::number(self.from)),
            ("summary", Value::string(&self.summary)),
            ("ceiling", self.ceiling.map_or(Value::Null, Value::number)),
            (
                "input_tokens",
                self.input_tokens.map_or(Value::Null, Value::number),
            ),
            ("measured_end", Value::number(self.measured_end)),
            (
                "token_calibration",
                self.calibration.map_or(Value::Null, |(tokens, bytes)| {
                    Value::object([
                        ("tokens", Value::number(tokens)),
                        ("serialized_bytes", Value::number(bytes)),
                    ])
                }),
            ),
            ("reply_tokens", Value::number(self.reply_tokens)),
            (
                "summary_output_tokens",
                Value::number(self.summary_output_tokens),
            ),
            ("preview_until", Value::number(self.preview_until)),
            ("preview_limit", Value::number(self.preview_limit)),
            ("request_limit", Value::number(self.request_limit)),
        ])
    }
    pub fn parse(value: Option<&Value>) -> Result<Self, String> {
        let Some(value) = value else {
            return Ok(Self::default());
        };
        let number = |key| {
            value
                .get(key)
                .and_then(Value::as_usize)
                .ok_or_else(|| format!("Invalid saved context {key}"))
        };
        let optional = |key| match value.get(key) {
            Some(Value::Null) => Ok(None),
            Some(number) => number
                .as_usize()
                .map(Some)
                .ok_or("Invalid saved context measurement"),
            None => Err("Missing saved context measurement"),
        };
        Ok(Self {
            from: number("from")?,
            summary: value
                .get("summary")
                .and_then(Value::as_str)
                .ok_or("Invalid context summary")?
                .into(),
            ceiling: optional("ceiling")?,
            input_tokens: optional("input_tokens")?,
            measured_end: number("measured_end")?,
            calibration: match value.get("token_calibration") {
                None | Some(Value::Null) => None,
                Some(calibration) => Some((
                    calibration
                        .get("tokens")
                        .and_then(Value::as_usize)
                        .filter(|value| *value > 0)
                        .ok_or("Invalid saved token calibration")?,
                    calibration
                        .get("serialized_bytes")
                        .and_then(Value::as_usize)
                        .filter(|value| *value > 0)
                        .ok_or("Invalid saved byte calibration")?,
                )),
            },
            reply_tokens: number("reply_tokens")?,
            summary_output_tokens: value.get("summary_output_tokens").map_or(Ok(0), |value| {
                value
                    .as_usize()
                    .ok_or("Invalid saved summary output budget")
            })?,
            preview_until: value.get("preview_until").map_or(Ok(0), |value| {
                value
                    .as_usize()
                    .ok_or("Invalid saved context preview boundary")
            })?,
            preview_limit: value.get("preview_limit").map_or(Ok(0), |value| {
                value
                    .as_usize()
                    .ok_or("Invalid saved context preview limit")
            })?,
            request_limit: value.get("request_limit").map_or(Ok(0), |value| {
                value
                    .as_usize()
                    .ok_or("Invalid saved context request limit")
            })?,
        })
    }
    pub fn validate(&self, messages: &[Value]) -> Result<(), String> {
        if self.from == 0
            || self.from > messages.len()
            || self.measured_end > messages.len()
            || (self.from > 1 && self.summary.trim().is_empty())
            || self.ceiling == Some(0)
            || self
                .calibration
                .is_some_and(|(tokens, bytes)| tokens == 0 || bytes == 0)
            || self.preview_until > messages.len()
            || self.preview_limit > crate::tools::OUTPUT_LIMIT
            || messages
                .get(self.from)
                .is_some_and(|message| message.get("role").and_then(Value::as_str) == Some("tool"))
        {
            return Err("Invalid saved context boundary".into());
        }
        Ok(())
    }
    pub fn reset_usage(&mut self) {
        // Summary allowance belongs to the saved model/effort, not this live projection.
        self.input_tokens = None;
        self.measured_end = 0;
        self.ceiling = None;
        self.reply_tokens = 0;
        self.calibration = None;
    }
    pub fn observe(&mut self, usage: Option<&Value>, end: usize) {
        self.input_tokens = usage
            .and_then(|usage| usage.get("prompt_tokens"))
            .and_then(Value::as_usize)
            .filter(|tokens| *tokens > 0);
        self.measured_end = if self.input_tokens.is_some() { end } else { 0 };
        self.reply_tokens = usage
            .and_then(|usage| usage.get("completion_tokens"))
            .and_then(Value::as_usize)
            .unwrap_or(0);
        if self.input_tokens.is_none() {
            // A later request cannot reuse tokens measured for a different projection.
            self.calibration = None;
        }
    }
    pub fn calibrate(&mut self, serialized_bytes: usize) {
        if let Some(tokens) = self.input_tokens
            && serialized_bytes > 0
        {
            self.calibration = Some((tokens, serialized_bytes));
        }
    }
    pub(crate) fn estimate_bytes(&self, size: usize) -> usize {
        let Some((tokens, bytes)) = self.calibration else {
            return size;
        };
        // Use three times the observed token density, capped at the original
        // byte upper bound. This also bounds newly appended history growth.
        let numerator = tokens.saturating_mul(3).min(bytes);
        if numerator == bytes {
            return size;
        }
        size.saturating_mul(numerator).saturating_add(bytes - 1) / bytes
    }
    pub(crate) fn summary_byte_budget(&self, token_budget: usize) -> usize {
        let Some((tokens, bytes)) = self.calibration else {
            return token_budget;
        };
        // Summary requests contain already observed history and no tool schema.
        // Keep twice the measured density; live growth retains its larger margin.
        // Overflow or an unusable measurement falls back to one byte per token.
        let numerator = tokens.saturating_mul(2).min(bytes);
        token_budget
            .checked_mul(bytes)
            .and_then(|scaled| scaled.checked_div(numerator))
            .unwrap_or(token_budget)
    }
    pub fn project(&self, messages: &[Value], compatible_from: usize) -> Vec<Value> {
        let mut result = Vec::new();
        if let Some(system) = messages.first() {
            result.push(system.clone());
        }
        if let Some(requests) = view::requests(messages, self.from, self.request_budget()) {
            result.push(requests);
        }
        if !self.summary.is_empty() {
            result.push(Value::object([
                ("role", Value::string("user")),
                (
                    "content",
                    Value::string(format!(
                        "{SUMMARY_PREFIX} It covers the conversation before history:{}; read history:N for any original message and history:requests for every user request.\n\n{}",
                        self.from, self.summary
                    )),
                ),
            ]));
        }
        result.extend(
            messages
                .iter()
                .enumerate()
                .skip(self.from)
                .map(|(index, message)| self.project_message(message, index, compatible_from)),
        );
        result
    }

    pub(crate) fn project_message(
        &self,
        message: &Value,
        index: usize,
        compatible_from: usize,
    ) -> Value {
        if index < self.preview_until
            && self.preview_limit > 0
            && message.get("role").and_then(Value::as_str) == Some("tool")
        {
            view::preview(message, index, self.preview_limit)
        } else if index < compatible_from
            && message.get("role").and_then(Value::as_str) == Some("assistant")
        {
            portable(message)
        } else {
            message.clone()
        }
    }

    fn request_budget(&self) -> usize {
        // Sessions saved before the request budget existed keep the former bound.
        if self.request_limit > 0 {
            self.request_limit
        } else {
            8192
        }
    }

    pub(crate) fn user_requests(&self, messages: &[Value]) -> String {
        view::requests(messages, messages.len(), self.request_budget())
            .and_then(|value| {
                value
                    .get("content")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            })
            .unwrap_or_default()
    }
    pub fn estimate(&self, messages: &[Value], compatible_from: usize) -> usize {
        if let Some(tokens) = self.input_tokens
            && self.measured_end >= self.from
            && self.measured_end <= messages.len()
        {
            return tokens
                .saturating_add(self.estimate_bytes(bytes(&messages[self.measured_end..])));
        }
        self.estimate_bytes(
            bytes(&self.project(messages, compatible_from))
                .saturating_add(crate::tools::definitions().encode().len()),
        )
    }
    pub fn budgets(&self, limits: Limits) -> (usize, usize) {
        let capacity = self.ceiling.unwrap_or(limits.context).min(limits.context);
        // Derive the reserve from this model's window and observed response size.
        let reserve = (capacity / 8)
            .max(self.reply_tokens.saturating_mul(2))
            .min(capacity / 3)
            .min(limits.output.unwrap_or(usize::MAX))
            .max(1);
        (capacity.saturating_sub(reserve), reserve)
    }
}
pub fn portable(message: &Value) -> Value {
    match message {
        Value::Object(fields) => Value::Object(
            fields
                .iter()
                .filter(|(key, _)| {
                    matches!(
                        key.as_str(),
                        "role" | "content" | "tool_calls" | "tool_call_id"
                    )
                })
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect(),
        ),
        _ => message.clone(),
    }
}
pub fn bytes(messages: &[Value]) -> usize {
    messages.iter().fold(0usize, |total, message| {
        total.saturating_add(message.encode().len())
    })
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod request_view_tests;
