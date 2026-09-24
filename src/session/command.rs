//! Direct command execution, streamed observations and an exact canonical receipt.
use super::{Event, worker::Context};
use crate::{
    command,
    json::{self, Value},
    tools::Output,
    workspace::{Budget, Workspace},
};
use std::{
    sync::atomic::Ordering,
    time::{Duration, Instant},
};

pub(super) fn execute(
    script: &str,
    path: &str,
    timeout: u64,
    shell: &command::Shell,
    workspace: &Workspace,
    context: &Context,
) -> (Output, Event) {
    let id = context.next_effect.fetch_add(1, Ordering::Relaxed);
    let (output, success) = dispatch(id, script, path, timeout, shell, workspace, context);
    let finished = Event::CommandFinished {
        id,
        summary: output.summary.clone(),
        success,
        failed: output.failed,
    };
    (output, finished)
}

fn dispatch(
    id: u64,
    script: &str,
    path: &str,
    timeout: u64,
    shell: &command::Shell,
    workspace: &Workspace,
    context: &Context,
) -> (Output, bool) {
    let budget = Budget {
        cancelled: &context.cancelled,
        deadline: Instant::now() + Duration::from_secs(10),
    };
    let proposal =
        match command::prepare_with_shell(workspace, script, path, timeout, shell, &budget) {
            Ok(proposal) => proposal,
            Err(error) => return (Output::not_executed(error), false),
        };
    let planned = context.send(
        Event::CommandPlanned {
            id,
            preview: proposal.preview.clone(),
        },
        true,
    );
    #[cfg(test)]
    if planned.is_continue() {
        context.before_effect("run_command");
    }
    if planned.is_break() || context.check().is_err() {
        return (
            Output::not_executed("command cancelled before launch; not executed"),
            false,
        );
    }
    let output_deadline = Instant::now() + Duration::from_secs(timeout);
    let budget = Budget {
        cancelled: &context.cancelled,
        deadline: output_deadline,
    };
    let mut live_output_omitted = false;
    let result = command::run_with_start(
        proposal,
        workspace,
        &budget,
        &mut || {
            let _ = context.notify(Event::CommandStarted { id });
        },
        &mut |channel, text| {
            if !context.notify(Event::CommandOutput {
                id,
                channel,
                text: text.into(),
            }) {
                live_output_omitted = true;
            }
            std::ops::ControlFlow::Continue(())
        },
    );
    match result {
        Ok(mut result) => {
            result.truncated |= live_output_omitted;
            let success = result.success();
            let summary = result.summary();
            let exit = result.exit;
            let mut output = Output::success(
                json::object([
                    ("ok", Value::Bool(success)),
                    ("status", Value::String(result.stop.name().into())),
                    ("executed", Value::Bool(true)),
                    (
                        "exit_code",
                        exit.and_then(|e| e.code)
                            .map_or(Value::Null, |n| Value::Number(n.to_string())),
                    ),
                    (
                        "signal",
                        exit.and_then(|e| e.signal)
                            .map_or(Value::Null, |n| Value::Number(n.to_string())),
                    ),
                    ("stdout", Value::String(result.stdout)),
                    ("stderr", Value::String(result.stderr)),
                    ("truncated", Value::Bool(result.truncated)),
                    ("output_bytes", Value::Number(result.bytes.to_string())),
                    ("elapsed_ms", Value::Number(result.elapsed_ms.to_string())),
                    ("cleanup_confirmed", Value::Bool(!result.cleanup_failed)),
                ]),
                summary,
                result.truncated,
            );
            output.failed = !success;
            (output, success)
        }
        Err(error) => {
            // OS errors are reported by kind/code, never echo an environment or credentials.
            let message = if cfg!(windows) && error.kind() == std::io::ErrorKind::Unsupported {
                format!("Command not executed · {error}")
            } else {
                format!(
                    "Command could not start · {:?}{}",
                    error.kind(),
                    error
                        .raw_os_error()
                        .map_or(String::new(), |code| format!(" (OS {code})"))
                )
            };
            (Output::not_executed(&message), false)
        }
    }
}
