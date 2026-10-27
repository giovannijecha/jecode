//! Launches a structured command in a conversation-owned native terminal.
use super::{exec, processes, shell};
use crate::{cancel::Cancellation, json::Value};
use std::time::Duration;

pub(super) fn execute(
    context: shell::Context<'_>,
    arguments: &Value,
    cancellation: &Cancellation,
) -> Result<Value, String> {
    let _ = shell::protect_paths(arguments)?;
    if arguments.get("stdin").is_some() {
        return Err("Use process write for terminal input; stdin applies to non-TTY exec".into());
    }
    let (mut command, _, label) = exec::command(context.root, context.temporary, arguments)?;
    if !command.get_envs().any(|(key, _)| key == "TERM") {
        command.env("TERM", "xterm-256color");
    }
    let size = crate::pty::size(arguments)?;
    let wait = processes::wait_argument(arguments)?.unwrap_or(Duration::ZERO);
    let output_limit = processes::output_limit(arguments)?;
    let timeout = arguments
        .get("timeout_seconds")
        .map(|value| {
            value
                .as_usize()
                .filter(|n| *n > 0)
                .map(|n| Duration::from_secs(n as u64))
                .ok_or("timeout_seconds must be a positive integer")
        })
        .transpose()?;
    let logs = context.outputs.create()?;
    let streams = [
        processes::Stream {
            path: context.outputs.resolve(&logs.stdout_ref)?,
            reference: logs.stdout_ref,
        },
        processes::Stream {
            path: context.outputs.resolve(&logs.stderr_ref)?,
            reference: logs.stderr_ref,
        },
    ];
    let (started, control) = match crate::pty::start(
        &command,
        size,
        cancellation,
        [logs.stdout, logs.stderr],
    ) {
        Ok(started) => started,
        Err(error) if !error.started => return Err(error.message),
        Err(error) => {
            return Ok(Value::object([
                (
                    "error",
                    Value::string(format!(
                        "The terminal bridge started but its control channel was unavailable: {}. Inspect the workspace and saved output before retrying.",
                        error.message
                    )),
                ),
                ("outcome", Value::string("unknown")),
                ("stdout_file", Value::string(&streams[0].reference)),
                ("stderr_file", Value::string(&streams[1].reference)),
            ]));
        }
    };
    context.processes.adopt_terminal(
        &label,
        streams,
        started,
        processes::Adoption {
            timeout,
            wait,
            output_limit,
            cancellation,
        },
        control,
    )
}

#[cfg(test)]
#[path = "interactive_tests.rs"]
mod tests;
