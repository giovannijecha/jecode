//! Provider measurements anchor the request; only unmeasured text is estimated.

use super::{Context, Value, portable};
use crate::{attachments::Inputs, backend::Usage};

/// Request-local numbers captured before inference, while source history is locked.
pub(crate) struct RequestMeasurement {
    end: usize,
    bytes: usize,
    overhead: usize,
    media: usize,
    fresh: bool,
    baseline: Option<(usize, usize)>,
}

impl Context {
    pub fn clear_measurement(&mut self) {
        self.input_tokens = None;
        self.output_tokens = None;
        self.output_message = None;
        self.measured_end = 0;
        self.measured_bytes = None;
        self.measured_overhead_bytes = None;
    }

    pub fn reset_usage(&mut self) {
        self.reset_projection();
        self.reported = super::provider::Counter::default();
    }

    pub fn reset_projection(&mut self) {
        self.clear_measurement();
        self.ceiling = None;
        self.calibration = None;
    }

    /// Missing usage leaves the last measured prefix and its density intact.
    pub fn observe(&mut self, usage: Option<&Usage>, end: usize) -> bool {
        let Some(tokens) = usage
            .and_then(|usage| usage.input_tokens)
            .filter(|n| *n > 0)
        else {
            return false;
        };
        self.clear_measurement();
        self.input_tokens = Some(tokens);
        self.output_tokens = usage.and_then(|usage| usage.output_tokens);
        self.measured_end = end;
        true
    }

    #[cfg(test)]
    pub fn calibrate(&mut self, serialized_bytes: usize) {
        if let Some(tokens) = self.input_tokens
            && serialized_bytes > 0
        {
            self.calibration = Some((tokens, serialized_bytes));
        }
    }

    pub(crate) fn request_measurement(
        &self,
        source: &[Value],
        request: &[Value],
        inputs: Inputs,
        delegation: bool,
        overhead: usize,
    ) -> RequestMeasurement {
        let media = media_tokens(request, inputs);
        RequestMeasurement {
            end: source.len(),
            bytes: text_bytes(request).saturating_add(crate::tools::encoded_size_for(delegation)),
            overhead,
            media,
            fresh: !request
                .iter()
                .any(|message| message.get("role").and_then(Value::as_str) == Some("assistant")),
            baseline: self.growth_baseline(source, media, inputs),
        }
    }

    /// Observes the actual request, never pairing old usage with a new payload.
    pub(crate) fn record_request(&mut self, usage: Option<&Usage>, request: RequestMeasurement) {
        let Some(tokens) = usage
            .and_then(|usage| usage.input_tokens)
            .filter(|n| *n > 0)
        else {
            return;
        };
        let RequestMeasurement {
            end,
            bytes,
            overhead,
            media,
            fresh,
            baseline,
        } = request;
        let sample = baseline.and_then(|(base_tokens, base_bytes)| {
            let growth_tokens = tokens.checked_sub(base_tokens)?;
            let growth_bytes = bytes.checked_sub(base_bytes)?;
            (growth_tokens > 0 && growth_bytes > 0).then_some((growth_tokens, growth_bytes))
        });
        if let Some(sample) = sample {
            self.learn_density(sample);
        } else if self.calibration.is_none() && fresh && bytes > 0 && tokens > media {
            // A fresh prompt has no replayed reasoning to contaminate text density.
            self.calibration = Some((tokens - media, bytes));
        }
        self.observe(usage, end);
        self.measured_bytes = Some(bytes);
        self.measured_overhead_bytes = Some(overhead);
    }

    fn learn_density(&mut self, sample: (usize, usize)) {
        // Tiny framing changes must not outweigh a recent substantial text sample.
        // Bound the retained weight so a different workload can still recalibrate.
        self.calibration = Some(self.calibration.map_or(sample, |(tokens, bytes)| {
            let weight = bytes.min(64 * 1024);
            let retained = (tokens as u128 * weight as u128).div_ceil(bytes as u128);
            (
                sample
                    .0
                    .saturating_add(usize::try_from(retained).unwrap_or(usize::MAX)),
                sample.1.saturating_add(weight),
            )
        }));
    }

    fn growth_baseline(
        &self,
        source: &[Value],
        media: usize,
        inputs: Inputs,
    ) -> Option<(usize, usize)> {
        if media > 0 || self.retired_views(source, inputs) > 0 || self.measured_end < self.from {
            return None;
        }
        let recent = source.get(self.measured_end..)?;
        let assistant = recent.first()?;
        if assistant.get("role").and_then(Value::as_str) != Some("assistant")
            || recent
                .iter()
                .skip(1)
                .any(|message| message.get("role").and_then(Value::as_str) == Some("assistant"))
        {
            return None;
        }
        // The prior response's provider token count includes its opaque reasoning.
        // Remove it from both sides before learning the new tool/user text density.
        let base_tokens = self.input_tokens?.checked_add(self.output_tokens?)?;
        let base_bytes = self
            .measured_bytes?
            .checked_add(portable(assistant).encode().len())?;
        Some((base_tokens, base_bytes))
    }

    pub(crate) fn estimate_bytes(&self, size: usize) -> usize {
        let Some((tokens, bytes)) = self.calibration else {
            // An explicit approximation until a provider measurement is available.
            return size.div_ceil(4);
        };
        let tokens = tokens.min(bytes);
        let scaled = (size as u128 * tokens as u128).div_ceil(bytes as u128);
        usize::try_from(scaled).unwrap_or(usize::MAX)
    }

    pub(crate) fn adjust_system(&self, tokens: usize, before: usize, after: usize) -> usize {
        if after >= before {
            tokens.saturating_add(self.estimate_bytes(after - before))
        } else {
            tokens.saturating_sub(self.estimate_bytes(before - after))
        }
    }

    pub(crate) fn estimate_projected_for(
        &self,
        messages: &[Value],
        inputs: Inputs,
        delegation: bool,
    ) -> usize {
        let mut bytes = crate::tools::encoded_size_for(delegation);
        let mut generated = 0usize;
        for message in messages {
            if let Some(tokens) = crate::conversation::output_tokens(message) {
                generated = generated.saturating_add(tokens);
            } else {
                bytes = bytes.saturating_add(portable(message).encode().len());
            }
        }
        self.estimate_bytes(bytes)
            .saturating_add(generated)
            .saturating_add(media_tokens(messages, inputs))
    }

    pub(crate) fn summary_byte_budget(&self, token_budget: usize) -> usize {
        let Some((tokens, bytes)) = self.calibration else {
            return token_budget.saturating_mul(2);
        };
        // Packing summaries keeps its own margin; request growth uses observed density.
        let numerator = tokens.saturating_mul(2).min(bytes);
        token_budget
            .checked_mul(bytes)
            .and_then(|scaled| scaled.checked_div(numerator))
            .unwrap_or(token_budget)
    }

    pub(crate) fn estimate_growth(
        &self,
        messages: &[Value],
        tokens: usize,
        inputs: Inputs,
    ) -> usize {
        let active_start = crate::attachments::active_view_start(messages);
        let mut generated = 0usize;
        let mut bytes = 0usize;
        let mut media = 0usize;
        for (index, message) in messages.iter().enumerate().skip(self.measured_end) {
            if self.returned.contains(&index) {
                continue;
            }
            if let Some(output) = self.output_tokens
                && index == self.output_message.unwrap_or(self.measured_end)
                && message.get("role").and_then(Value::as_str) == Some("assistant")
            {
                generated = generated.saturating_add(output);
            } else if let Some(output) = crate::conversation::output_tokens(message) {
                generated = generated.saturating_add(output);
            } else {
                bytes = bytes.saturating_add(portable(message).encode().len());
                media = media.saturating_add(media_for(message, index >= active_start, inputs));
            }
        }
        tokens
            .saturating_add(generated)
            .saturating_add(self.estimate_bytes(bytes))
            .saturating_add(media)
    }

    pub(crate) fn measured_input(&self, messages: &[Value], inputs: Inputs) -> Option<usize> {
        let tokens = self.input_tokens?;
        (self.measured_end >= self.from && self.measured_end <= messages.len())
            .then(|| tokens.saturating_sub(self.retired_views(messages, inputs)))
    }

    fn retired_views(&self, messages: &[Value], inputs: Inputs) -> usize {
        if crate::attachments::active_view_start(messages) <= self.measured_end {
            return 0;
        }
        let measured = &messages[..self.measured_end.min(messages.len())];
        measured[crate::attachments::active_view_start(measured)..]
            .iter()
            .filter(|message| message.get("role").and_then(Value::as_str) == Some("tool"))
            .map(|message| crate::attachments::weight_for(message, inputs))
            .fold(0usize, usize::saturating_add)
    }
}

fn text_bytes(messages: &[Value]) -> usize {
    messages
        .iter()
        .map(|message| portable(message).encode().len())
        .fold(0usize, usize::saturating_add)
}

fn media_tokens(messages: &[Value], inputs: Inputs) -> usize {
    let active_start = crate::attachments::active_view_start(messages);
    messages
        .iter()
        .enumerate()
        .map(|(index, message)| media_for(message, index >= active_start, inputs))
        .fold(0usize, usize::saturating_add)
}

fn media_for(message: &Value, active: bool, inputs: Inputs) -> usize {
    if active || message.get("role").and_then(Value::as_str) != Some("tool") {
        crate::attachments::weight_for(message, inputs)
    } else {
        0
    }
}

#[cfg(test)]
mod tests;
