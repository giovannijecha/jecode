//! The only model-facing loop: ordered reads and effects with exact receipts.
use super::{
    End, Event, Failure, Metrics, Model, generation,
    history::{History, MAX_REQUEST, Receipt, Step},
    worker::{Backend, Context, operation_deadline},
};
use crate::{
    providers::openai_account::Status,
    tools::{Output, Prepared},
    workspace::{Budget, Workspace},
};
use std::time::{Duration, Instant};

#[allow(clippy::too_many_arguments)] // One task loop; the injected clock keeps deadline tests deterministic.
pub(super) fn run(
    backend: &mut impl Backend,
    history: &mut History,
    context: &Context,
    model: Model,
    workspace: Option<&Workspace>,
    started: Instant,
    clock: impl Fn() -> Instant,
    metrics: &mut Metrics,
) -> Result<End, Failure> {
    loop {
        context.check()?;
        super::queue::take(history, context)?;
        super::context::ensure(
            backend,
            history,
            context,
            model,
            workspace.is_some(),
            metrics,
        )?;
        let request = history.request(model, workspace.is_some())?;
        if metrics.requests != 0 && context.send(Event::RequestStarted, true).is_break() {
            return Err(Failure::Cancelled);
        }
        let turn = history.turns.last_mut().ok_or(Failure::Worker)?;
        let result =
            generation::generate(backend, &request, turn, context, started, &clock, metrics);
        history.checkpoint()?;
        result?;
        let turn = history.turns.last_mut().ok_or(Failure::Worker)?;
        let step = turn.steps.last_mut().ok_or(Failure::Worker)?;
        let response = step.response.as_ref().ok_or(Failure::Worker)?;
        match response.status {
            Status::Incomplete => return Ok(End::Incomplete),
            Status::Refused => return Ok(End::Refused),
            Status::Completed if response.tool_calls.is_empty() => {
                if super::queue::take(history, context)? {
                    continue;
                }
                return Ok(End::Complete);
            }
            Status::Completed => {}
        }
        context.check()?;
        let workspace = workspace.ok_or(Failure::UnexpectedTools)?;
        execute(backend, history, workspace, context, model, &clock, metrics)?;
    }
}

fn execute(
    backend: &mut impl Backend,
    history: &mut History,
    workspace: &Workspace,
    context: &Context,
    model: Model,
    clock: &impl Fn() -> Instant,
    metrics: &mut Metrics,
) -> Result<(), Failure> {
    let count = current(history)?.results.len();
    let shell = history.shell.clone();
    for index in 0..count {
        context.check()?;
        let call = &current(history)?
            .response
            .as_ref()
            .ok_or(Failure::Worker)?
            .tool_calls[index];
        let call_id = call.id.clone();
        let prepared = Prepared::parse(&call.name, &call.arguments);
        let (name, path) = match &prepared {
            Ok(tool) => (tool.name(), tool.path().to_owned()),
            Err(_) => ("rejected tool", String::new()),
        };
        let effect = prepared
            .as_ref()
            .is_ok_and(|tool| tool.changes_file() || matches!(tool, Prepared::Command { .. }));
        if !effect
            && context
                .send(Event::ToolStarted { name, path }, true)
                .is_break()
        {
            return Err(Failure::Cancelled);
        }
        metrics.tool_calls = metrics.tool_calls.saturating_add(1);
        if effect {
            let receipt = &mut current(history)?.results[index];
            receipt.output = crate::json::encode(&crate::json::object([
                ("ok", crate::json::Value::Bool(false)),
                ("status", crate::json::Value::String("uncertain".into())),
                ("error", crate::json::Value::String("tool outcome is unknown after interruption; inspect the workspace before repeating any effect".into())),
            ]), crate::tools::MAX_OUTPUT).expect("bounded uncertainty receipt");
            receipt.summary = format!("{name} / outcome unknown after interruption");
            history.checkpoint()?;
        }
        let (mut output, completion, mut image) = match prepared {
            Ok(Prepared::Recall {
                turn,
                step,
                receipt,
                offset,
            }) => {
                let output = super::receipt_recall::execute(
                    history,
                    turn,
                    step,
                    receipt,
                    offset,
                    &Budget {
                        cancelled: &context.cancelled,
                        deadline: operation_deadline(clock(), Duration::from_secs(10)),
                    },
                );
                (output, None, None)
            }
            Ok(Prepared::Image { path, image_id }) => {
                let (output, image) = super::image_tool::execute(
                    history,
                    workspace,
                    path.as_deref(),
                    image_id.as_deref(),
                    context,
                )?;
                (output, None, image)
            }
            Ok(Prepared::Command {
                command,
                path,
                timeout_seconds,
            }) => {
                let (output, event) = super::command::execute(
                    &command,
                    &path,
                    timeout_seconds,
                    &shell,
                    workspace,
                    context,
                );
                (output, Some(event), None)
            }
            Ok(tool) if tool.changes_file() => {
                let recoveries = history.recovery_store().map_err(|_| Failure::Storage)?;
                let session_id = history.record.as_ref().map(|record| record.id().to_owned());
                let (output, event) = super::edit::execute(
                    tool,
                    workspace,
                    context,
                    &recoveries,
                    session_id.as_deref(),
                    &call_id,
                );
                (output, Some(event), None)
            }
            Ok(tool) => (
                tool.execute(
                    workspace,
                    &Budget {
                        cancelled: &context.cancelled,
                        deadline: operation_deadline(clock(), Duration::from_secs(10)),
                    },
                ),
                None,
                None,
            ),
            Err(error) => (Output::error(error), None, None),
        };
        if image.is_some() {
            context.check()?;
            if !admit_image(
                backend,
                history,
                context,
                model,
                metrics,
                index,
                &output,
                image.as_ref().ok_or(Failure::Worker)?,
            )? {
                output = Output::error(
                    "view_image was rejected: this image plus pending visual evidence and context exceed the 8 MiB encoded account request limit. Existing successful views remain pending. Save a smaller PNG and ask to view its path; a saved image_id can be viewed again only if it fits. For a previously blocked session, use /discard-pending-images before requesting replacement views.",
                );
                image = None;
            }
        }
        let stop_after = output.stop_after;
        let receipt = &mut current(history)?.results[index];
        receipt.summary = format!("{name} / {}", output.summary);
        receipt.output = output.text;
        receipt.image = image;
        // The exact effect receipt reaches storage before its final presentation.
        // Waiting for channel capacity here cannot delay process supervision or
        // cleanup, and keeps every completion ahead of the next operation.
        let checkpoint = history.checkpoint();
        #[cfg(test)]
        if checkpoint.is_ok()
            && let Some(observed) = &history.test_outcome_checkpoint
        {
            observed.store(true, std::sync::atomic::Ordering::Release);
        }
        if let Some(event) = completion {
            let _ = context.send(event, false);
        }
        checkpoint?;
        if stop_after {
            return Err(Failure::Storage);
        }
        if !effect {
            let _ = context.send(
                Event::ToolFinished {
                    summary: output.summary,
                    failed: output.failed,
                    limited: output.limited,
                },
                false,
            );
        }
        context.check()?;
    }
    Ok(())
}

/// Stage only the proposed receipt in memory, then measure the exact provider
/// body. A rejected view never becomes a successful canonical receipt.
fn candidate_fits(
    history: &mut History,
    model: Model,
    index: usize,
    output: &Output,
    image: &crate::image::Evidence,
) -> Result<bool, Failure> {
    // The remaining calls in this batch can each still produce a bounded text
    // receipt. Reserve their worst escaped wire size before pinning pixels.
    let remaining = current(history)?.results.len().saturating_sub(index + 1);
    let Some(limit) = remaining
        .checked_mul(2 * crate::tools::MAX_OUTPUT)
        .and_then(|reserved| MAX_REQUEST.checked_sub(reserved))
    else {
        return Ok(false);
    };
    let proposed = Receipt {
        call_id: current(history)?.results[index].call_id.clone(),
        output: output.text.clone(),
        summary: format!("view_image / {}", output.summary),
        image: Some(image.clone()),
    };
    let old = std::mem::replace(&mut current(history)?.results[index], proposed);
    let measured = history
        .projected_request(model, true)
        .map(|request| request.encode(limit));
    current(history)?.results[index] = old;
    match measured? {
        Ok(_) => Ok(true),
        Err(crate::providers::openai_account::Error::Json(crate::json::Error::Limit)) => Ok(false),
        Err(_) => Err(Failure::HistoryLimit),
    }
}

#[allow(clippy::too_many_arguments)] // The controller owns the provider, receipt and context cursor.
fn admit_image(
    backend: &mut impl Backend,
    history: &mut History,
    context: &Context,
    model: Model,
    metrics: &mut Metrics,
    index: usize,
    output: &Output,
    image: &crate::image::Evidence,
) -> Result<bool, Failure> {
    loop {
        if candidate_fits(history, model, index, output, image)? {
            return Ok(true);
        }
        if !super::context::eligible_before_current_step(history) {
            return Ok(false);
        }
        let before = history.projected_cursor();
        super::context::compact(backend, history, context, model, true, metrics)?;
        if before == history.projected_cursor() {
            return Ok(false);
        }
    }
}
fn current(history: &mut History) -> Result<&mut Step, Failure> {
    history
        .turns
        .last_mut()
        .and_then(|turn| turn.steps.last_mut())
        .ok_or(Failure::Worker)
}
