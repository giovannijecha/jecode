//! Consume exactly one model request; a transport failure is never retried here.
use super::{
    Event, Failure, Metrics,
    history::{MAX_TEXT, Receipt, Step, Turn},
    worker::{Backend, Context, failure, millis, operation_deadline},
};
use crate::{
    providers::openai_account::{Progress, Request},
    tls::Budget,
    tools::Output,
};
use std::{
    ops::ControlFlow,
    time::{Duration, Instant},
};

pub(super) fn generate(
    backend: &mut impl Backend,
    request: &Request,
    turn: &mut Turn,
    context: &Context,
    started: Instant,
    clock: &impl Fn() -> Instant,
    metrics: &mut Metrics,
) -> Result<(), Failure> {
    context.check()?;
    turn.steps.push(Step::default());
    let step = turn.steps.last_mut().ok_or(Failure::Worker)?;
    let mut limit = false;
    let mut thinking = false;
    metrics.requests = metrics.requests.saturating_add(1);
    let result = backend.generate(
        request,
        &Budget {
            deadline: operation_deadline(clock(), Duration::from_secs(600)),
            cancelled: &context.cancelled,
        },
        &mut |progress| {
            if let Progress::Attempt(attempt) = progress {
                metrics.observe(&attempt);
                if attempt.retrying {
                    let _ = context.send(Event::Retrying, true);
                }
                step.attempts.push(attempt);
                return ControlFlow::Continue(());
            }
            if context.check().is_err() {
                return ControlFlow::Break(());
            }
            let text = match progress {
                Progress::Text(text) | Progress::Reasoning(text) => text,
                Progress::Attempt(_) => unreachable!(),
            };
            if text.len() > MAX_TEXT.saturating_sub(step.text.len() + step.reasoning.len()) {
                limit = true;
                return ControlFlow::Break(());
            }
            match progress {
                Progress::Text(text) => {
                    step.text.push_str(text);
                    if !text.is_empty() && metrics.first_text_ms.is_none() {
                        metrics.first_text_ms = Some(millis(started));
                    }
                    context.text(text, true)
                }
                Progress::Reasoning(text) => {
                    step.reasoning.push_str(text);
                    if !thinking {
                        thinking = true;
                        context.send(Event::Thinking, true)
                    } else {
                        ControlFlow::Continue(())
                    }
                }
                Progress::Attempt(_) => unreachable!(),
            }
        },
    );
    let response = match result {
        Ok(response) => response,
        Err(error) => {
            // An attempt without usage makes the total unknown, not zero or the
            // subtotal from earlier requests in this turn.
            metrics.usage(&Default::default());
            return Err(if limit {
                Failure::OutputLimit
            } else {
                failure(error, context)
            });
        }
    };
    metrics.usage(&response.usage);
    step.results = response
        .tool_calls
        .iter()
        .map(|call| Receipt {
            call_id: call.id.clone(),
            output: Output::error("tool was not executed because the turn stopped").text,
            summary: "Not executed".into(),
        })
        .collect();
    // Keep validated terminal facts even if presentation cannot accept them.
    step.response = Some(response);
    let response = step.response.as_ref().ok_or(Failure::Worker)?;
    if limit || response.text.len() > MAX_TEXT.saturating_sub(step.reasoning.len()) {
        return Err(Failure::OutputLimit);
    }
    let suffix = response.text.strip_prefix(&step.text);
    if let Some(suffix) = suffix.filter(|suffix| !suffix.is_empty()) {
        if metrics.first_text_ms.is_none() {
            metrics.first_text_ms = Some(millis(started));
        }
        // A validated final answer survives a simultaneous late cancellation.
        // Tool execution and every following request still check cancellation.
        let _ = context.text(suffix, false);
    } else if suffix.is_none() {
        // Per-part terminal suffixes or unindexed deltas can make the visible
        // stream differ internally. The provider has validated the terminal
        // output against each indexed part (or the unindexed raw stream).
        let _ = context.send(Event::TextReconciled(response.text.clone()), false);
    }
    step.text.clone_from(&response.text);
    step.accepted = true;
    Ok(())
}
