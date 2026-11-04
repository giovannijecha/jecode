use super::{Agent, timing::RequestTiming};
use crate::{
    backend::{Completion, Failure, FailureKind, Limits, Update, Usage},
    effort::Effort,
    events::Event,
    json::Value,
};
use std::time::{Duration, Instant};

/// What a provider request is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Purpose {
    /// The conversation: tool definitions, the selected effort and live usage.
    Conversation,
    /// A summary over the conversation's own request: the same definitions and
    /// effort keep the provider's cached prefix; recorded as maintenance.
    CachedSummary,
    /// Standalone maintenance without tools, at maintenance effort.
    Maintenance,
}

impl Purpose {
    fn tools(self) -> bool {
        self != Self::Maintenance
    }

    fn label(self) -> &'static str {
        if self == Self::Conversation {
            "completion"
        } else {
            "context_compaction"
        }
    }
}

/// Retry state shared by the attempts of one request.
#[derive(Default)]
pub(super) struct Retries {
    attempt: usize,
    empty_responses: usize,
}

impl Agent {
    pub(super) fn complete_retry(
        &self,
        messages: &[Value],
        purpose: Purpose,
        output_tokens: Option<usize>,
        emit: &mut impl FnMut(Event) -> Result<(), String>,
    ) -> Result<Completion, Failure> {
        let mut retries = Retries::default();
        loop {
            if let Some(completion) = self.complete_attempt(
                messages,
                purpose,
                output_tokens,
                &mut retries,
                RequestTiming::start(),
                emit,
            )? {
                return Ok(completion);
            }
        }
    }

    /// Sends one request. `None` means a temporary failure has been recorded
    /// and waited out, so the caller may retry, with a rebuilt request if needed.
    pub(super) fn complete_attempt(
        &self,
        messages: &[Value],
        purpose: Purpose,
        output_tokens: Option<usize>,
        retries: &mut Retries,
        mut timing: RequestTiming,
        emit: &mut impl FnMut(Event) -> Result<(), String>,
    ) -> Result<Option<Completion>, Failure> {
        if self.cancellation.requested() {
            return Err("Operation cancelled".into());
        }
        let mut partial = String::new();
        let mut reported_usage = Usage::default();
        let mut has_usage = false;
        let tools = purpose.tools();
        let effort = if tools {
            self.effort()
        } else {
            self.client.maintenance_effort(Effort::Medium)
        };
        let mut text = super::streaming::Text::new(self.redactor.clone());
        let result = self.client.sample_with_effort(
            effort,
            messages,
            tools,
            output_tokens,
            self.tools.attachments(),
            &mut |update| {
                if !matches!(&update, Update::Waiting { .. } | Update::Usage(_))
                    && !matches!(&update, Update::TextDelta(delta) if delta.trim().is_empty())
                {
                    timing.first_update.get_or_insert_with(Instant::now);
                }
                match update {
                    Update::Waiting {
                        request_bytes: bytes,
                    } => {
                        timing.request_bytes = Some(bytes);
                        let notifying = Instant::now();
                        let notified = emit(Event::Waiting {
                            model: self.redact(self.model()),
                        });
                        timing.notification += notifying.elapsed();
                        if notified.is_ok() && !self.cancellation.requested() {
                            timing.dispatched.get_or_insert_with(Instant::now);
                        }
                        notified
                    }
                    Update::TextDelta(delta) => {
                        partial.push_str(&delta);
                        if let Some(text) = text.push(&delta) {
                            emit(Event::Streaming { text })?;
                        }
                        Ok(())
                    }
                    Update::Reasoning => emit(Event::Reasoning),
                    Update::Working => emit(Event::Working),
                    Update::Usage(usage) => {
                        has_usage = true;
                        reported_usage.update(usage);
                        if purpose == Purpose::Conversation
                            && self
                                .context
                                .reported
                                .observe(self.identity(), &reported_usage)
                        {
                            self.emit_context(emit)?;
                        }
                        Ok(())
                    }
                }
            },
        );
        let mut final_context_error = None;
        match &result {
            Ok(completion) => {
                self.record_charges(completion);
                // Backends without streaming usage still report their final measurement.
                if let Some(usage) = completion.usage {
                    has_usage = true;
                    reported_usage.update(usage);
                    if purpose == Purpose::Conversation
                        && self
                            .context
                            .reported
                            .observe(self.identity(), &reported_usage)
                    {
                        final_context_error = self.emit_context(emit).err();
                    }
                }
            }
            Err(error) => {
                if let Some(charges) = &error.charges {
                    self.charges.lock().unwrap().include(charges);
                }
            }
        }
        self.record_request_timing(
            purpose.label(),
            retries.attempt.saturating_add(1),
            timing,
            effort,
            has_usage.then_some(reported_usage),
            match (&result, self.cancellation.requested()) {
                (Ok(_), _) => "complete",
                (Err(_), true) => "cancelled",
                (Err(_), false) => "failed",
            },
        );
        if let Some(error) = final_context_error {
            return Err(error.into());
        }
        if let Some(text) = text.finish() {
            emit(Event::Streaming { text })?;
        }
        match result {
            Ok(mut completion) => {
                decode_json_text_arguments(&mut completion);
                Ok(Some(completion))
            }
            Err(error) if matches!(error.kind, FailureKind::Temporary | FailureKind::Empty) => {
                if error.kind == FailureKind::Empty {
                    retries.empty_responses += 1;
                    if retries.empty_responses >= 3 {
                        // Stays `Empty`: a cached summary falls back to portions.
                        return Err(Failure {
                            message: format!(
                                "The backend returned three empty responses. No tools from these responses were executed; the original session is preserved. {}",
                                self.redact(&error.message)
                            ),
                            ..error
                        });
                    }
                }
                retries.attempt = retries.attempt.saturating_add(1);
                let attempt = retries.attempt;
                let delay = retry_delay(attempt, error.retry_after, cfg!(test));
                self.record_recovery(purpose.label(), attempt, delay, &error.message, &partial)?;
                emit(Event::Recovering {
                    attempt,
                    delay,
                    error: self.redact(&error.message),
                    reason: error.reason,
                })?;
                self.cancellation.wait(delay)?;
                emit(Event::RecoveryFinished)?;
                Ok(None)
            }
            Err(mut error) => {
                error.message = self.redact(&error.message);
                if error.kind == FailureKind::Terminal
                    && let Some(reason) = error.reason
                {
                    emit(Event::BackendFailure {
                        reason,
                        diagnostic: self
                            .redact(error.diagnostic.as_deref().unwrap_or(&error.message)),
                    })?;
                }
                if error.kind == FailureKind::Context {
                    self.record_recovery(
                        "context_rejected",
                        0,
                        Duration::ZERO,
                        &error.message,
                        &partial,
                    )?;
                }
                Err(error)
            }
        }
    }

    pub(super) fn model_limits(
        &mut self,
        events: &mut impl crate::events::EventSink,
    ) -> Result<Option<Limits>, String> {
        let mut attempt = 0usize;
        loop {
            if self.cancellation.requested() {
                return Err("Operation cancelled".into());
            }
            match self.client.prepare_turn() {
                Ok(limits) => {
                    if self.cancellation.requested() {
                        return Err("Operation cancelled".into());
                    }
                    return Ok(limits);
                }
                Err(error) if error.kind == FailureKind::Temporary => {
                    attempt = attempt.saturating_add(1);
                    let delay = retry_delay(attempt, error.retry_after, cfg!(test));
                    self.record_recovery("model_metadata", attempt, delay, &error.message, "")?;
                    events.emit(Event::Recovering {
                        attempt,
                        delay,
                        error: self.redact(&error.message),
                        reason: error.reason,
                    })?;
                    self.cancellation.wait(delay)?;
                    events.emit(Event::RecoveryFinished)?;
                }
                Err(error) => {
                    let message = self.redact(&error.message);
                    if error.kind == FailureKind::Terminal
                        && let Some(reason) = error.reason
                    {
                        events.emit(Event::BackendFailure {
                            reason,
                            diagnostic: self
                                .redact(error.diagnostic.as_deref().unwrap_or(&message)),
                        })?;
                    }
                    return Err(message);
                }
            }
        }
    }

    fn record_recovery(
        &self,
        operation: &str,
        attempt: usize,
        delay: Duration,
        error: &str,
        partial: &str,
    ) -> Result<(), String> {
        let after_message = self.messages.lock().unwrap().len();
        self.events.lock().unwrap().push(Value::object([
            ("type", Value::string("request_recovery")),
            ("operation", Value::string(operation)),
            ("attempt", Value::number(attempt)),
            ("delay_ms", Value::number(delay.as_millis())),
            ("error", Value::string(self.redact(error))),
            ("partial_text", Value::string(self.redact(partial))),
            ("after_message", Value::number(after_message)),
        ]));
        self.checkpoint(crate::sessions::Stage::Preserve)
    }

    fn record_request_timing(
        &self,
        operation: &str,
        attempt: usize,
        timing: RequestTiming,
        effort: Effort,
        usage: Option<Usage>,
        status: &str,
    ) {
        let after_message = self.messages.lock().unwrap().len();
        self.events.lock().unwrap().push(timing.event(
            operation,
            (self.backend_id(), &self.redact(self.model()), effort),
            attempt,
            status,
            usage,
            after_message,
        ));
    }
}

/// Decodes JSON-text arrays and objects where a tool's schema expects them
/// (`tools::coerce`), in the calls to run and in the stored message alike.
fn decode_json_text_arguments(completion: &mut Completion) {
    for call in &mut completion.calls {
        let Some(decoded) = crate::tools::coerce::arguments(&call.name, &call.arguments) else {
            continue;
        };
        if let Value::Object(fields) = &mut completion.message
            && let Some(Value::Array(stored)) = fields.get_mut("tool_calls")
        {
            for entry in stored {
                if entry.get("id").and_then(Value::as_str) != Some(call.id.as_str()) {
                    continue;
                }
                let target = match entry {
                    Value::Object(entry) if entry.contains_key("function") => {
                        entry.get_mut("function")
                    }
                    entry => Some(entry),
                };
                if let Some(Value::Object(target)) = target {
                    target.insert("arguments".into(), Value::string(&decoded));
                }
            }
        }
        call.arguments = decoded;
    }
}

pub(super) fn retry_delay(attempt: usize, server: Option<Duration>, fixture: bool) -> Duration {
    let ordinary =
        Duration::from_secs(2u64.saturating_mul(1u64 << attempt.saturating_sub(1).min(5)));
    let ordinary = if fixture {
        Duration::from_millis(10)
    } else {
        ordinary
    };
    ordinary.max(server.unwrap_or_default())
}
