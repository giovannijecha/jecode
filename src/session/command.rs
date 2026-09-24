//! One-use command approval, streamed observations and an exact canonical receipt.
use super::{Event, Metrics, approval, worker::Context};
use crate::{
    command,
    json::{self, Value},
    tools::Output,
    workspace::{Budget, Workspace},
};
use std::{sync::atomic::Ordering, time::Instant};

#[allow(clippy::too_many_arguments)] // Mirrors one tool call, its scope and the turn owner.
pub(super) fn execute(
    script: &str,
    path: &str,
    timeout: u64,
    shell: &command::Shell,
    workspace: &Workspace,
    context: &Context,
    deadline: Instant,
    denied: &mut bool,
    metrics: &mut Metrics,
) -> Output {
    let id = context.next_approval.fetch_add(1, Ordering::Relaxed);
    let (output, success) = dispatch(
        id, script, path, timeout, shell, workspace, context, deadline, denied, metrics,
    );
    let _ = context.send(
        Event::CommandFinished {
            id,
            summary: output.summary.clone(),
            success,
            failed: output.failed,
        },
        false,
    );
    output
}
#[allow(clippy::too_many_arguments)]
fn dispatch(
    id: u64,
    script: &str,
    path: &str,
    timeout: u64,
    shell: &command::Shell,
    workspace: &Workspace,
    context: &Context,
    deadline: Instant,
    denied: &mut bool,
    metrics: &mut Metrics,
) -> (Output, bool) {
    if *denied {
        return (
            Output::error(
                "further effects are disabled after a denial; wait for a new user request",
            ),
            false,
        );
    }
    let budget = Budget {
        cancelled: &context.cancelled,
        deadline,
    };
    let proposal =
        match command::prepare_with_shell(workspace, script, path, timeout, shell, &budget) {
            Ok(proposal) => proposal,
            Err(error) => return (Output::error(error), false),
        };
    while context.decisions.try_recv().is_ok() {}
    if context
        .send(
            Event::CommandProposed {
                id,
                preview: proposal.preview.clone(),
            },
            true,
        )
        .is_break()
    {
        return (
            Output::error("command cancelled before approval; not executed"),
            false,
        );
    }
    match approval::wait(context, id, deadline, metrics) {
        Ok(true) => {}
        Ok(false) => {
            *denied = true;
            return (
                Output::success(
                    json::object([
                        ("ok", Value::Bool(false)),
                        ("status", Value::String("denied".into())),
                        ("approved", Value::Bool(false)),
                        ("executed", Value::Bool(false)),
                    ]),
                    "Denied · command not executed".into(),
                    false,
                ),
                false,
            );
        }
        Err(()) => {
            return (
                Output::error("approval cancelled or expired; command not executed"),
                false,
            );
        }
    }
    if context.send(Event::CommandStarted { id }, true).is_break() {
        return (
            Output::error("command cancelled before launch; not executed"),
            false,
        );
    }
    let output_deadline = deadline.min(Instant::now() + std::time::Duration::from_secs(timeout));
    let result = command::run(proposal, workspace, &budget, &mut |channel, text| {
        context.send_until(
            Event::CommandOutput {
                id,
                channel,
                text: text.into(),
            },
            output_deadline,
        )
    });
    match result {
        Ok(result) => {
            let success = result.success();
            let summary = result.summary();
            let exit = result.exit;
            let mut output = Output::success(
                json::object([
                    ("ok", Value::Bool(success)),
                    ("status", Value::String(result.stop.name().into())),
                    ("approved", Value::Bool(true)),
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
            (Output::error(&message), false)
        }
    }
}
