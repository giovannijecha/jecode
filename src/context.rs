use crate::{attachments::Inputs, backend::Limits, json::Value};
use std::collections::BTreeSet;

pub use crate::conversation::portable;

pub(crate) mod measurement;
pub(crate) mod provider;
pub(crate) mod source;
mod state;
mod view;

pub(crate) use state::CONTINUATION;
pub use state::{RequestState, SNAPSHOT_HEADER, state_message};
pub(crate) use view::preview;

/// Introduces a compaction summary, which the same model wrote in an earlier context window.
const SUMMARY_PREFIX: &str = "Another language model started this task and wrote the summary below. The work it did with tools is still in place: files, saved outputs and the original history. Build on it and avoid repeating finished work.";

/// Share of the effective context window at which proactive reduction starts.
pub const AUTO_COMPACT_DEFAULT: u8 = 80;

/// Validates an automatic compaction percentage: a whole number from 1 to 99.
pub fn auto_compact_percent(value: usize) -> Result<u8, String> {
    u8::try_from(value)
        .ok()
        .filter(|percent| (1..=99).contains(percent))
        .ok_or_else(|| "Auto-compact must be a whole percentage from 1 to 99".into())
}

/// Tokens at which `percent` of `capacity` is reached, rounded up.
pub fn threshold(capacity: usize, percent: u8) -> usize {
    let tokens = (capacity as u128 * u128::from(percent)).div_ceil(100);
    usize::try_from(tokens).unwrap_or(usize::MAX).max(1)
}

/// The context count for display: the last provider reading, or the estimate
/// of the next request while it differs from that reading.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Status {
    /// Provider input plus generated output, or its reported total; never a projection.
    pub tokens: Option<usize>,
    /// Estimate of the next request when tool results, input or compaction moved
    /// the window at least one point from the reading. Automatic compaction
    /// decides with the same estimate; the next provider reading replaces it.
    pub pending: Option<usize>,
    /// Context window from provider metadata or an exact documented model ID;
    /// None while unknown.
    pub capacity: Option<usize>,
    /// Configured automatic reduction threshold, in percent.
    pub auto_compact: u8,
}

impl Status {
    /// Whole percentage of the known context window, rounded down.
    pub fn percent(&self) -> Option<usize> {
        let capacity = self.capacity.filter(|capacity| *capacity > 0)?;
        let percent = self.pending.or(self.tokens)? as u128 * 100 / capacity as u128;
        Some(usize::try_from(percent).unwrap_or(usize::MAX))
    }

    /// Marks the next request's estimate when it moved a point from the reading.
    pub fn with_estimate(mut self, estimate: usize) -> Self {
        if let Some(capacity) = self.capacity.filter(|capacity| *capacity > 0) {
            let moved = self
                .tokens
                .is_none_or(|tokens| tokens.abs_diff(estimate).saturating_mul(100) >= capacity);
            self.pending = moved.then_some(estimate);
        }
        self
    }
}

/// A view of durable history. Compaction never deletes the source messages.
#[derive(Clone, Debug)]
pub struct Context {
    pub(crate) reported: provider::Counter,
    /// Derived from turn_returned events; source history keeps its stable indices.
    pub returned: BTreeSet<usize>,
    pub from: usize,
    pub summary: String,
    /// Operational references captured at compaction, independent of the model summary.
    pub skill_references: String,
    pub ceiling: Option<usize>,
    pub input_tokens: Option<usize>,
    /// Generated tokens of the measured response, reasoning included; they
    /// stand in for the size of the assistant message appended after it.
    pub output_tokens: Option<usize>,
    /// Position of the measured response, after any arrivals during inference.
    pub output_message: Option<usize>,
    pub measured_end: usize,
    /// Portable text and catalog bytes of the request behind the measurement.
    pub measured_bytes: Option<usize>,
    /// Its system, tool catalog and request-only continuation sizes.
    pub measured_overhead_bytes: Option<usize>,
    pub calibration: Option<(usize, usize)>,

    /// Output allowance learned from summary truncation, independent of live prompt usage.
    pub summary_output_tokens: usize,
    /// Byte budget for verbatim recent user requests after compaction.
    pub request_limit: usize,
    /// Runtime state updates in source order, each at the position first sent.
    pub states: Vec<RequestState>,
}
impl Default for Context {
    fn default() -> Self {
        Self {
            reported: provider::Counter::default(),
            returned: BTreeSet::new(),
            from: 1,
            summary: String::new(),
            skill_references: String::new(),
            ceiling: None,
            input_tokens: None,
            output_tokens: None,
            output_message: None,
            measured_end: 0,
            measured_bytes: None,
            measured_overhead_bytes: None,
            calibration: None,

            summary_output_tokens: 0,
            request_limit: 8192,
            states: Vec::new(),
        }
    }
}
impl Context {
    pub fn value(&self) -> Value {
        Value::object([
            ("provider_usage", self.reported.value()),
            ("from", Value::number(self.from)),
            ("summary", Value::string(&self.summary)),
            ("skill_references", Value::string(&self.skill_references)),
            ("ceiling", self.ceiling.map_or(Value::Null, Value::number)),
            (
                "input_tokens",
                self.input_tokens.map_or(Value::Null, Value::number),
            ),
            (
                "output_tokens",
                self.output_tokens.map_or(Value::Null, Value::number),
            ),
            ("measured_end", Value::number(self.measured_end)),
            (
                "output_message",
                self.output_message.map_or(Value::Null, Value::number),
            ),
            (
                "measured_bytes",
                self.measured_bytes.map_or(Value::Null, Value::number),
            ),
            (
                "measured_overhead_bytes",
                self.measured_overhead_bytes
                    .map_or(Value::Null, Value::number),
            ),
            (
                "token_calibration",
                self.calibration.map_or(Value::Null, |(tokens, bytes)| {
                    Value::object([
                        ("tokens", Value::number(tokens)),
                        ("serialized_bytes", Value::number(bytes)),
                        ("method", Value::string("text_growth")),
                    ])
                }),
            ),
            (
                "summary_output_tokens",
                Value::number(self.summary_output_tokens),
            ),
            ("request_limit", Value::number(self.request_limit)),
            ("request_states", state::value(&self.states)),
        ])
    }
    pub fn parse(value: Option<&Value>) -> Result<Self, String> {
        let value = value.ok_or("Missing saved context")?;
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
            reported: provider::Counter::parse(Some(
                value
                    .get("provider_usage")
                    .ok_or("Missing saved provider usage")?,
            ))?,
            returned: BTreeSet::new(),
            from: number("from")?,
            summary: value
                .get("summary")
                .and_then(Value::as_str)
                .ok_or("Invalid context summary")?
                .into(),
            skill_references: value
                .get("skill_references")
                .and_then(Value::as_str)
                .ok_or("Invalid saved skill references")?
                .into(),
            ceiling: optional("ceiling")?,
            input_tokens: optional("input_tokens")?,
            output_tokens: optional("output_tokens")?,
            measured_end: number("measured_end")?,
            output_message: optional("output_message")?,
            measured_bytes: optional("measured_bytes")?,
            measured_overhead_bytes: optional("measured_overhead_bytes")?,
            calibration: match value.get("token_calibration") {
                Some(Value::Null) => None,
                Some(calibration) => {
                    let tokens = calibration
                        .get("tokens")
                        .and_then(Value::as_usize)
                        .filter(|value| *value > 0)
                        .ok_or("Invalid saved token calibration")?;
                    let bytes = calibration
                        .get("serialized_bytes")
                        .and_then(Value::as_usize)
                        .filter(|value| *value > 0)
                        .ok_or("Invalid saved byte calibration")?;
                    if calibration.get("method").and_then(Value::as_str) != Some("text_growth") {
                        return Err("Invalid saved token calibration method".into());
                    }
                    Some((tokens, bytes))
                }
                None => return Err("Missing saved token calibration".into()),
            },
            summary_output_tokens: number("summary_output_tokens")?,
            request_limit: number("request_limit")?,
            states: state::parse(value.get("request_states"))?,
        })
    }
    pub fn validate(&self, messages: &[Value]) -> Result<(), String> {
        if self.from == 0
            || self.from > messages.len()
            || self.measured_end > messages.len()
            || self
                .output_message
                .is_some_and(|index| index < self.measured_end || index >= messages.len())
            || self.measured_bytes == Some(0)
            || (self.from > 1 && self.summary.trim().is_empty())
            || self.request_limit == 0
            || self.ceiling == Some(0)
            || self
                .calibration
                .is_some_and(|(tokens, bytes)| tokens == 0 || bytes == 0)
            || messages
                .get(self.from)
                .is_some_and(|message| message.get("role").and_then(Value::as_str) == Some("tool"))
            || self
                .states
                .windows(2)
                .any(|pair| pair[0].before > pair[1].before)
            || self.states.iter().any(|state| {
                state.before > messages.len()
                    || messages.get(state.before).is_some_and(|message| {
                        message.get("role").and_then(Value::as_str) == Some("tool")
                    })
            })
        {
            return Err("Invalid saved context boundary".into());
        }
        Ok(())
    }

    pub fn restore_returned(&mut self, events: &[Value], count: usize) {
        self.returned.clear();
        for event in events {
            if event.get("type").and_then(Value::as_str) == Some("turn_returned")
                && let (Some(from), Some(end)) = (
                    event.get("from_message").and_then(Value::as_usize),
                    event.get("after_message").and_then(Value::as_usize),
                )
            {
                self.returned.extend(from.min(count)..end.min(count));
            }
        }
    }
    pub fn project(&self, messages: &[Value], compatible_from: usize) -> Vec<Value> {
        let mut result = Vec::new();
        if let Some(system) = messages.first() {
            result.push(system.clone());
        }
        if let Some(requests) =
            view::requests(messages, self.from, self.request_budget(), &self.returned)
        {
            result.push(requests);
        }
        if !self.summary.is_empty() {
            result.push(Value::object([
                ("role", Value::string("user")),
                (
                    "content",
                    Value::string(format!(
                        "{SUMMARY_PREFIX} It covers the conversation before history:{}; messages from that boundary onward remain in the transcript below. Read history:N for any original message and history:requests for every user request.\n\n{}{}",
                        self.from, self.summary,
                        if self.skill_references.is_empty() { String::new() } else { format!("\n\n{}", self.skill_references) }
                    )),
                ),
            ]));
        }
        let mut states = self.visible_states().peekable();
        for (index, message) in messages.iter().enumerate().skip(self.from) {
            while let Some(state) = states.next_if(|state| state.before <= index) {
                result.push(state_message(&state.text));
            }
            if !self.returned.contains(&index) {
                result.push(self.project_message(message, index, compatible_from));
            }
        }
        result.extend(states.map(|state| state_message(&state.text)));
        result
    }

    pub(crate) fn project_message(
        &self,
        message: &Value,
        index: usize,
        compatible_from: usize,
    ) -> Value {
        if index < compatible_from
            && message.get("role").and_then(Value::as_str) == Some("assistant")
            && !crate::conversation::has_origin(message)
        {
            portable(message)
        } else {
            crate::conversation::without_stale_reasoning(message, self.from)
        }
    }

    fn request_budget(&self) -> usize {
        self.request_limit
    }

    pub(crate) fn user_requests(&self, messages: &[Value]) -> String {
        view::requests(
            messages,
            messages.len(),
            self.request_budget(),
            &self.returned,
        )
        .and_then(|value| {
            value
                .get("content")
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
        .unwrap_or_default()
    }
    /// The effective window: declared capacity lowered by any ceiling learned
    /// from backend rejections, or that ceiling alone without catalog metadata.
    pub fn capacity(&self, limits: Option<Limits>) -> Option<usize> {
        match (self.ceiling, limits) {
            (Some(ceiling), Some(limits)) => Some(ceiling.min(limits.context)),
            (ceiling, limits) => ceiling.or(limits.map(|limits| limits.context)),
        }
    }

    /// The input budget for proactive reduction: `percent` of the effective window.
    /// Output allowance is calculated separately from the provider limits.
    pub fn budget(&self, limits: Limits, percent: u8) -> usize {
        threshold(
            self.capacity(Some(limits)).unwrap_or(limits.context),
            percent,
        )
    }
}

#[cfg(test)]
mod returned_tests;
pub fn bytes(messages: &[Value]) -> usize {
    bytes_for(messages, Inputs::default())
}

pub fn bytes_for(messages: &[Value], inputs: Inputs) -> usize {
    let active_start = crate::attachments::view_start(messages, inputs);
    messages
        .iter()
        .enumerate()
        .map(|(index, message)| bytes_for_message(message, index >= active_start, inputs))
        .fold(0usize, usize::saturating_add)
}

pub(crate) fn bytes_for_message(message: &Value, active_view: bool, inputs: Inputs) -> usize {
    // User attachments persist; tool views expire after the next assistant
    // reply unless the backend retains them.
    let media = if active_view || message.get("role").and_then(Value::as_str) != Some("tool") {
        crate::attachments::weight_for(message, inputs)
    } else {
        0
    };
    portable(message).encode().len().saturating_add(media)
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod request_view_tests;
