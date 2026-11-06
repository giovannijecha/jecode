use super::RunError;
use crate::cancel::Cancellation;
use crate::sandbox::MARKER;
use std::ffi::OsStr;
use std::io::{self, Read};
use std::os::unix::process::CommandExt;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

mod input;

/// The guardian restores these names once the policy holds; see `restore_deferred`.
const DEFERRED: &str = "JECODE_INTERNAL_DEFERRED_";

/// Variables the dynamic loader acts on before the guardian's `main`.
fn loader_variable(name: &OsStr) -> bool {
    let name = name.as_encoded_bytes();
    name.starts_with(b"LD_") || name.starts_with(b"DYLD_")
}

pub(super) struct Spawned {
    pub child: Child,
    pub stdout: ChildStdout,
    pub writer: Option<JoinHandle<io::Result<()>>>,
    pub owner: Owner,
    pub stdin: Option<std::fs::File>,
}

pub(super) struct Owner {
    control: Option<ChildStdin>,
    stop: Arc<AtomicBool>,
}

impl Drop for Owner {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        // Only the owner holds this write end. No target inherits it.
        drop(self.control.take());
    }
}

/// The sandbox policy carried by the trusted marker. An empty marker carries
/// no policy and is treated exactly like an absent one.
pub(crate) fn sandbox_policy(command: &Command) -> Option<&OsStr> {
    command
        .get_envs()
        .find(|(key, _)| *key == MARKER)
        .and_then(|(_, value)| value)
        .filter(|value| !value.is_empty())
}

pub(super) fn spawn(
    command: &Command,
    bytes: Option<Vec<u8>>,
    cancellation: &Cancellation,
) -> Result<Spawned, RunError> {
    spawn_input(command, bytes, cancellation, false)
}

pub(super) fn spawn_duplex(
    command: &Command,
    cancellation: &Cancellation,
) -> Result<Spawned, RunError> {
    spawn_input(command, None, cancellation, true)
}

fn spawn_input(
    command: &Command,
    bytes: Option<Vec<u8>>,
    cancellation: &Cancellation,
    persistent: bool,
) -> Result<Spawned, RunError> {
    let pipe = (bytes.is_some() || persistent)
        .then(input::Pipe::open)
        .transpose()?;
    let executable = crate::native::helper("process_unix").map_err(before_start)?;
    let mut helper = Command::new(executable);
    let confined = sandbox_policy(command).is_some();
    if confined {
        // The target inherits the guardian's environment. `get_envs` below lists
        // only what the policy selected, so nothing else may come from this process.
        helper.env_clear();
    }
    helper
        .arg("guard")
        .arg(
            pipe.as_ref()
                .map_or(std::path::Path::new("/dev/null"), |pipe| pipe.path()),
        )
        .arg(command.get_program())
        .args(command.get_args())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0);
    if let Some(directory) = command.get_current_dir() {
        helper.current_dir(directory);
    }
    for (key, value) in command.get_envs() {
        if let Some(value) = value {
            if confined && loader_variable(key) {
                let mut carried = std::ffi::OsString::from(DEFERRED);
                carried.push(key);
                helper.env(carried, value);
            } else {
                helper.env(key, value);
            }
        } else {
            helper.env_remove(key);
        }
    }
    helper
        .env_remove("BASH_ENV")
        .env_remove("ENV")
        .env_remove("SHELLOPTS")
        .env_remove("BASHOPTS");
    let mut child = helper
        .spawn()
        .map_err(|error| before_start(format!("Could not start Unix process guardian: {error}")))?;
    let owner = Owner {
        control: child.stdin.take(),
        stop: Arc::new(AtomicBool::new(false)),
    };
    let mut stdout = child.stdout.take().unwrap();
    let (sender, receiver) = mpsc::sync_channel(1);
    let handshake = thread::spawn(move || {
        let mut header = [0; 6];
        let result = stdout.read_exact(&mut header).map(|_| header);
        let _ = sender.send((stdout, result));
    });
    let started = Instant::now();
    loop {
        match receiver.recv_timeout(Duration::from_millis(10)) {
            Ok((stdout, Ok(header))) if header == *b"READY\n" => {
                let _ = handshake.join();
                let (pipe, stdin) = if persistent {
                    let stdin = match pipe.unwrap().into_writer() {
                        Ok(stdin) => stdin,
                        Err(error) => {
                            let _ = terminate(&mut child);
                            return Err(error);
                        }
                    };
                    (None, Some(stdin))
                } else {
                    (pipe, None)
                };
                let writer = pipe.zip(bytes).map(|(pipe, bytes)| {
                    pipe.send(bytes, Arc::clone(&owner.stop), cancellation.clone())
                });
                return Ok(Spawned {
                    child,
                    stdout,
                    writer,
                    owner,
                    stdin,
                });
            }
            Ok((_, Ok(header))) if header == *b"NOBOX\n" => {
                let _ = terminate(&mut child);
                let _ = handshake.join();
                let mut detail = String::new();
                if let Some(mut stderr) = child.stderr.take() {
                    let _ = stderr.read_to_string(&mut detail);
                }
                return Err(before_start(confinement_refused(detail.trim())));
            }
            Ok(_) => {
                let _ = terminate(&mut child);
                let _ = handshake.join();
                return Err(before_start("Could not start process: executable was not found or guardian initialization failed".into()));
            }
            Err(mpsc::RecvTimeoutError::Timeout)
                if started.elapsed() < Duration::from_secs(5) && !cancellation.requested() => {}
            Err(_) => {
                let _ = terminate(&mut child);
                let _ = handshake.join();
                return Err(before_start(if cancellation.requested() {
                    "Operation cancelled".into()
                } else {
                    "Unix process guardian did not initialize".into()
                }));
            }
        }
    }
}

fn confinement_refused(detail: &str) -> String {
    let requirement = if cfg!(target_os = "linux") {
        "Workspace confinement needs Linux 5.13 or later with Landlock enabled. "
    } else {
        ""
    };
    format!(
        "Workspace commands cannot be confined on this system: {detail}. {requirement}Choose /permissions full to run without confinement."
    )
}

fn before_start(message: String) -> RunError {
    RunError {
        started: false,
        message,
    }
}

pub(super) fn terminate(child: &mut Child) -> Result<(), String> {
    let group = child.id().to_string();
    // The root is killed below even when the group cleanup cannot start.
    let result = (|| {
        let mut cleanup = Command::new(crate::native::helper("process_unix")?);
        cleanup
            .args(["kill-group", &group])
            .env("LC_ALL", "C")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped());
        let mut cleanup = cleanup
            .spawn()
            .map_err(|error| format!("Could not start process-group cleanup: {error}"))?;
        let started = Instant::now();
        let status = loop {
            match cleanup.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) if started.elapsed() < Duration::from_secs(2) => {
                    thread::sleep(Duration::from_millis(10));
                }
                Ok(None) => {
                    let _ = cleanup.kill();
                    let _ = cleanup.wait();
                    return Err("Process-group cleanup did not finish; outcome is unknown".into());
                }
                Err(error) => {
                    let _ = cleanup.kill();
                    let _ = cleanup.wait();
                    return Err(format!("Could not wait for process-group cleanup: {error}"));
                }
            }
        };
        if status.success() {
            return Ok(());
        }
        let mut details = String::new();
        if let Some(mut stderr) = cleanup.stderr.take() {
            let _ = stderr.read_to_string(&mut details);
        }
        if details.contains("No such process") {
            return Ok(());
        }
        Err(format!(
            "Process-group cleanup failed (exit {}): {}; outcome is unknown",
            status
                .code()
                .map_or("signal".into(), |code| code.to_string()),
            details.trim()
        ))
    })();
    if child.kill().is_ok() {
        let _ = child.wait();
    }
    result
}

#[cfg(test)]
mod tests;
