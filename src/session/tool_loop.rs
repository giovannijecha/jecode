//! The only model-facing loop: ordered reads and approved edits with exact receipts.
use super::{
    End, Event, Failure, Metrics, Model, generation,
    history::{History, Step},
    worker::{Backend, Context},
};
use crate::{
    providers::openai_account::Status,
    tools::{Output, Prepared},
    workspace::{Budget, Workspace},
};
use std::time::{Duration, Instant};

const MAX_REQUESTS: u32 = 8;
const MAX_TOOLS: u32 = 32;

pub(super) fn run(
    backend: &mut impl Backend,
    history: &mut History,
    context: &Context,
    model: Model,
    workspace: Option<&Workspace>,
    started: Instant,
    metrics: &mut Metrics,
) -> Result<End, Failure> {
    let mut denied = false;
    loop {
        context.check()?;
        if metrics.requests >= MAX_REQUESTS {
            return Err(Failure::StepLimit);
        }
        super::queue::take(history, context)?;
        if metrics.requests == 0 {
            super::context::ensure(
                backend,
                history,
                context,
                model,
                workspace.is_some(),
                metrics,
            )?;
        }
        let request = history.request(model, workspace.is_some())?;
        if metrics.requests != 0 && context.send(Event::RequestStarted, true).is_break() {
            return Err(Failure::Cancelled);
        }
        let turn = history.turns.last_mut().ok_or(Failure::Worker)?;
        let result = generation::generate(backend, &request, turn, context, started, metrics);
        history.checkpoint()?;
        result?;
        let turn = history.turns.last_mut().ok_or(Failure::Worker)?;
        let step = turn.steps.last_mut().ok_or(Failure::Worker)?;
        let response = step.response.as_ref().ok_or(Failure::Worker)?;
        match response.status {
            Status::Incomplete => return Ok(End::Incomplete),
            Status::Refused => return Ok(End::Refused),
            Status::Completed if response.tool_calls.is_empty() => {
                if metrics.requests < MAX_REQUESTS && super::queue::take(history, context)? {
                    continue;
                }
                return Ok(End::Complete);
            }
            Status::Completed => {}
        }
        context.check()?;
        let workspace = workspace.ok_or(Failure::UnexpectedTools)?;
        if metrics.requests >= MAX_REQUESTS
            || response.tool_calls.len() > (MAX_TOOLS - metrics.tool_calls) as usize
        {
            return Err(Failure::StepLimit);
        }
        execute(history, workspace, context, started, metrics, &mut denied)?;
    }
}

fn execute(
    history: &mut History,
    workspace: &Workspace,
    context: &Context,
    started: Instant,
    metrics: &mut Metrics,
    denied: &mut bool,
) -> Result<(), Failure> {
    let count = current(history)?.results.len();
    for index in 0..count {
        context.check()?;
        let call = &current(history)?
            .response
            .as_ref()
            .ok_or(Failure::Worker)?
            .tool_calls[index];
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
        metrics.tool_calls += 1;
        if effect {
            let receipt = &mut current(history)?.results[index];
            receipt.output = Output::error("tool outcome is unknown after interruption; inspect the workspace before repeating any effect").text;
            receipt.summary = format!("{name} / outcome unknown after interruption");
            history.checkpoint()?;
        }
        let output = match prepared {
            Ok(Prepared::Command {
                command,
                path,
                timeout_seconds,
            }) => super::command::execute(
                &command,
                &path,
                timeout_seconds,
                workspace,
                context,
                started + Duration::from_secs(600),
                denied,
                metrics,
            ),
            Ok(tool) if tool.changes_file() => super::approval::execute(
                tool,
                workspace,
                context,
                started + Duration::from_secs(600),
                denied,
                metrics,
            ),
            Ok(tool) => tool.execute(
                workspace,
                &Budget {
                    cancelled: &context.cancelled,
                    deadline: (Instant::now() + Duration::from_secs(10))
                        .min(started + Duration::from_secs(600)),
                },
            ),
            Err(error) => Output::error(error),
        };
        let receipt = &mut current(history)?.results[index];
        receipt.summary = format!("{name} / {}", output.summary);
        receipt.output = output.text;
        history.checkpoint()?;
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
