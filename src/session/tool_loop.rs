//! The only model-facing loop: ordered reads and effects with exact receipts.
use super::{
    End, Event, Failure, Metrics, Model, generation,
    history::{History, Step},
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
        execute(history, workspace, context, &clock, metrics)?;
    }
}

fn execute(
    history: &mut History,
    workspace: &Workspace,
    context: &Context,
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
        let (output, completion) = match prepared {
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
                (output, Some(event))
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
                (output, Some(event))
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
            ),
            Err(error) => (Output::error(error), None),
        };
        let stop_after = output.stop_after;
        let receipt = &mut current(history)?.results[index];
        receipt.summary = format!("{name} / {}", output.summary);
        receipt.output = output.text;
        // The exact effect receipt reaches storage before its final presentation.
        // Waiting for channel capacity here cannot delay process supervision or
        // cleanup, and keeps every completion ahead of the next operation.
        let checkpoint = history.checkpoint();
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
fn current(history: &mut History) -> Result<&mut Step, Failure> {
    history
        .turns
        .last_mut()
        .and_then(|turn| turn.steps.last_mut())
        .ok_or(Failure::Worker)
}
