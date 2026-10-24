use super::Agent;
use crate::{
    backend::{Completion, Failure, FailureKind, Limits, Update},
    events::Event,
    json::Value,
};
use std::time::Duration;

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
        tools: bool,
        output_tokens: Option<usize>,
        emit: &mut impl FnMut(Event) -> Result<(), String>,
    ) -> Result<Completion, Failure> {
        let mut retries = Retries::default();
        loop {
            if let Some(completion) =
                self.complete_attempt(messages, tools, output_tokens, &mut retries, emit)?
            {
                return Ok(completion);
            }
        }
    }

    /// Sends one request. `None` means a temporary failure has been recorded
    /// and waited out, so the caller may retry, with a rebuilt request if needed.
    pub(super) fn complete_attempt(
        &self,
        messages: &[Value],
        tools: bool,
        output_tokens: Option<usize>,
        retries: &mut Retries,
        emit: &mut impl FnMut(Event) -> Result<(), String>,
    ) -> Result<Option<Completion>, Failure> {
        if self.cancellation.requested() {
            return Err("Operation cancelled".into());
        }
        let mut partial = String::new();
        let mut text = super::streaming::Text::new(self.redactor.clone());
        let result = self.client.sample(
            messages,
            tools,
            output_tokens,
            self.tools.attachments(),
            &mut |update| match update {
                Update::TextDelta(delta) => {
                    partial.push_str(&delta);
                    if let Some(text) = text.push(&delta) {
                        emit(Event::Streaming { text })?;
                    }
                    Ok(())
                }
                Update::Reasoning => emit(Event::Reasoning),
                Update::Working => emit(Event::Working),
            },
        );
        match &result {
            Ok(completion) => self.record_charges(completion),
            Err(error) => {
                if let Some(charges) = &error.charges {
                    self.charges.lock().unwrap().include(charges);
                }
            }
        }
        if let Some(text) = text.finish() {
            emit(Event::Streaming { text })?;
        }
        match result {
            Ok(completion) => Ok(Some(completion)),
            Err(error) if matches!(error.kind, FailureKind::Temporary | FailureKind::Empty) => {
                if error.kind == FailureKind::Empty {
                    retries.empty_responses += 1;
                    if retries.empty_responses >= 3 {
                        return Err(format!(
                            "The backend returned three empty responses. No tools from these responses were executed; the original session is preserved. {}",
                            self.redact(&error.message)
                        ).into());
                    }
                }
                retries.attempt = retries.attempt.saturating_add(1);
                let attempt = retries.attempt;
                let delay = retry_delay(attempt, error.retry_after, cfg!(test));
                self.record_recovery(
                    if tools {
                        "completion"
                    } else {
                        "context_compaction"
                    },
                    attempt,
                    delay,
                    &error.message,
                    &partial,
                )?;
                emit(Event::Recovering {
                    attempt,
                    delay,
                    error: self.redact(&error.message),
                })?;
                self.cancellation.wait(delay)?;
                emit(Event::RecoveryFinished)?;
                Ok(None)
            }
            Err(mut error) => {
                error.message = self.redact(&error.message);
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
                    })?;
                    self.cancellation.wait(delay)?;
                    events.emit(Event::RecoveryFinished)?;
                }
                Err(error) => return Err(self.redact(&error.message)),
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
        self.events.lock().unwrap().push(Value::object([
            ("type", Value::string("request_recovery")),
            ("operation", Value::string(operation)),
            ("attempt", Value::number(attempt)),
            ("delay_ms", Value::number(delay.as_millis())),
            ("error", Value::string(self.redact(error))),
            ("partial_text", Value::string(self.redact(partial))),
            (
                "after_message",
                Value::number(self.messages.lock().unwrap().len()),
            ),
        ]));
        self.checkpoint(crate::sessions::Stage::Preserve)
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
