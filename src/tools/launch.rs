//! Persistent application launches: `exec` with `persistent: true`.
//!
//! An ordinary command belongs to the conversation: Jecode supervises its
//! whole process tree and ends it at its timeout, on stop, on cancellation and
//! when the conversation ends. A persistent launch is the explicit exception
//! for an application the user wants to keep using, such as a desktop app.
//! The native adapter starts it outside the command boundary: on Windows
//! detached from Jecode's console, in its own process group and outside its
//! enclosing job (or refuses the launch); on Unix in a new session through the
//! owned guardian helper. Its streams are the null device, so nothing it
//! writes can block on, or die with, Jecode's pipes.
//!
//! The call observes the application for at most `wait_ms` and reports
//! whether it was still running or had already exited. After that Jecode
//! keeps no handle on it: no timeout, cancellation or `process stop` applies,
//! and it outlives the conversation and Jecode itself.

use crate::cancel::Cancellation;
use crate::json::Value;
use std::process::Command;
#[cfg(unix)]
use std::process::{Child, Stdio};
use std::time::{Duration, Instant};

const DEFAULT_OBSERVATION: Duration = Duration::from_secs(2);
const MAX_OBSERVATION: Duration = Duration::from_secs(60);
const OWNERSHIP: &str = "Not owned by Jecode: no timeout, cancellation or process stop applies, and it keeps running after this call, the conversation and Jecode. Its output is discarded; use the application's own logs or windows.";
const FULL_ONLY: &str = "Persistent launches need the Full permission level: the application runs outside Workspace confinement and outlives Jecode's process ownership. The user can choose Full with /permissions; nothing ran";

/// Extra native facts for the launch result.
type Notes = Vec<(&'static str, Value)>;

#[cfg(windows)]
mod windows;

/// Whether these arguments request a persistent launch.
pub(super) fn requested(arguments: &Value) -> Result<bool, String> {
    match arguments.get("persistent") {
        None | Some(Value::Bool(false)) => Ok(false),
        Some(Value::Bool(true)) => Ok(true),
        Some(_) => Err("persistent must be a boolean".into()),
    }
}

/// The refusal for a level below Full, shared with the permission gate.
pub fn refusal() -> &'static str {
    FULL_ONLY
}

pub(super) fn execute(
    root: &std::path::Path,
    temporary: Option<&std::path::Path>,
    arguments: &Value,
    full: bool,
    cancellation: &Cancellation,
) -> Result<Value, String> {
    if !full {
        return Err(FULL_ONLY.into());
    }
    for (name, reason) in [
        ("tty", "a persistent application has no Jecode terminal"),
        ("stdin", "a persistent application reads the null device"),
        (
            "timeout_seconds",
            "Jecode does not own a persistent application",
        ),
        (
            "output_limit_bytes",
            "a persistent application's output is discarded",
        ),
    ] {
        if arguments.get(name).is_some() {
            return Err(format!(
                "{name} does not apply to persistent launches: {reason}"
            ));
        }
    }
    let observation = super::processes::wait_argument(arguments)?
        .unwrap_or(DEFAULT_OBSERVATION)
        .min(MAX_OBSERVATION);
    let (command, _, _) = super::exec::command(root, temporary, arguments)?;
    if cancellation.requested() {
        return Err("Operation cancelled; nothing was launched".into());
    }
    let started = Instant::now();
    #[cfg(windows)]
    {
        windows::launch(&command, observation, cancellation, started)
    }
    #[cfg(unix)]
    {
        let mut command = command;
        let (mut child, mut notes) = spawn(&mut command)?;
        let pid = child.id();
        let mut cancelled = false;
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break Some(status),
                Ok(None) => {}
                Err(error) => {
                    notes.push((
                        "error",
                        Value::string(format!("Could not observe the application: {error}")),
                    ));
                    break None;
                }
            }
            if cancellation.requested() {
                cancelled = true;
                break None;
            }
            let elapsed = started.elapsed();
            if elapsed >= observation {
                break None;
            }
            std::thread::sleep((observation - elapsed).min(Duration::from_millis(20)));
        };
        let elapsed = started.elapsed();
        if status.is_none() {
            release(child);
        }
        Ok(report(
            pid,
            status.map(|status| status.code()),
            elapsed,
            cancelled,
            notes,
        ))
    }
}

pub(super) fn report(
    pid: u32,
    status: Option<Option<i32>>,
    observed: Duration,
    cancelled: bool,
    notes: Notes,
) -> Value {
    let mut fields = vec![
        ("launch", Value::string("persistent")),
        ("pid", Value::number(pid)),
        (
            "application",
            Value::string(if status.is_some() {
                "exited"
            } else {
                "running"
            }),
        ),
        ("observed_ms", Value::number(observed.as_millis())),
        ("ownership", Value::string(OWNERSHIP)),
    ];
    if let Some(status) = status {
        fields.push(("exit_code", status.map_or(Value::Null, Value::number)));
    }
    if cancelled {
        // Cancellation ends the observation only; the launch already happened.
        fields.push(("wait_cancelled", Value::Bool(true)));
    }
    fields.extend(notes);
    Value::entries(fields)
}

/// Lets go of an application that is still running. Unix reaps it from a
/// background thread so it never lingers as a zombie of this process.
#[cfg(unix)]
fn release(child: Child) {
    let mut child = child;
    std::thread::spawn(move || {
        let _ = child.wait();
    });
}

#[cfg(unix)]
fn spawn(command: &mut Command) -> Result<(Child, Notes), String> {
    use std::io::Read;
    let helper = crate::native::helper("process_unix")
        .map_err(|error| format!("{error}; nothing was launched"))?;
    let mut launcher = Command::new(helper);
    launcher
        .arg("launch")
        .arg(command.get_program())
        .args(command.get_args())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    if let Some(directory) = command.get_current_dir() {
        launcher.current_dir(directory);
    }
    for (key, value) in command.get_envs() {
        match value {
            Some(value) => launcher.env(key, value),
            None => launcher.env_remove(key),
        };
    }
    let mut child = launcher.spawn().map_err(|error| {
        format!("Could not start the Unix launch helper: {error}; nothing was launched")
    })?;
    // End of file without a message means the application replaced the helper.
    let mut failure = String::new();
    let read = child.stdout.take().unwrap().read_to_string(&mut failure);
    if read.is_err() || !failure.trim().is_empty() {
        let _ = child.wait();
        return Err(format!(
            "Could not launch {}: {}; nothing was launched",
            command_name(command),
            failure.trim()
        ));
    }
    Ok((child, Vec::new()))
}

#[cfg(unix)]
fn command_name(command: &Command) -> String {
    command.get_program().to_string_lossy().into_owned()
}

#[cfg(test)]
mod tests;
