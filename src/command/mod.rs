//! Approved, non-interactive shell execution. Platform owners contain ordinary
//! descendants and join the child; neither a workspace path nor a job is a sandbox.
mod capture;
mod platform;
mod shell;
#[cfg(test)]
pub(crate) mod tests;

use crate::workspace::{Budget, Directory, Workspace};
use std::{
    io,
    ops::ControlFlow,
    thread,
    time::{Duration, Instant},
};
pub(crate) fn shell_name() -> &'static str {
    shell::NAME
}

#[derive(Clone, Debug)]
pub struct Preview {
    pub command: String,
    pub cwd: String,
    pub shell: &'static str,
    pub timeout_seconds: u64,
}
pub(crate) struct Proposal {
    pub preview: Preview,
    directory: Directory,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Channel {
    Stdout,
    Stderr,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stop {
    Exited,
    Cancelled,
    Timeout,
    OutputLimit,
    IoError,
}
impl Stop {
    pub fn name(self) -> &'static str {
        match self {
            Self::Exited => "exited",
            Self::Cancelled => "cancelled",
            Self::Timeout => "timed_out",
            Self::OutputLimit => "output_limit",
            Self::IoError => "io_error",
        }
    }
}
#[derive(Clone, Copy, Debug)]
pub struct Exit {
    pub code: Option<i64>,
    pub signal: Option<i32>,
}
pub struct Outcome {
    pub stop: Stop,
    pub exit: Option<Exit>,
    pub stdout: String,
    pub stderr: String,
    pub truncated: bool,
    pub bytes: u64,
    pub elapsed_ms: u64,
    pub cleanup_failed: bool,
}
impl Outcome {
    pub fn success(&self) -> bool {
        self.stop == Stop::Exited
            && self.exit.is_some_and(|e| e.code == Some(0))
            && !self.cleanup_failed
    }
    pub fn summary(&self) -> String {
        let state = match self.stop {
            Stop::Exited => "Command finished",
            Stop::Cancelled => "Command interrupted",
            Stop::Timeout => "Command timed out",
            Stop::OutputLimit => "Command stopped at output limit",
            Stop::IoError => "Command output failed",
        };
        let exit = self
            .exit
            .map_or(String::new(), |e| match (e.code, e.signal) {
                (Some(code), _) => format!(" · exit {code}"),
                (_, Some(signal)) => format!(" · signal {signal}"),
                _ => String::new(),
            });
        format!(
            "{state}{exit} · {:.1}s{}{}",
            self.elapsed_ms as f64 / 1000.0,
            if self.truncated {
                " · output shortened"
            } else {
                ""
            },
            if self.cleanup_failed {
                " · cleanup could not be confirmed"
            } else {
                ""
            }
        )
    }
}

pub(crate) fn prepare(
    workspace: &Workspace,
    command: &str,
    cwd: &str,
    timeout_seconds: u64,
    budget: &Budget<'_>,
) -> Result<Proposal, &'static str> {
    if command.trim().is_empty()
        || command.len() > 4096
        || !(1..=300).contains(&timeout_seconds)
        || command
            .chars()
            .any(|c| (c.is_control() && !matches!(c, '\n' | '\t')) || capture::invisible(c))
    {
        return Err(
            "command must be 1..4096 UTF-8 bytes without hidden controls; timeout_seconds must be 1..300",
        );
    }
    if cwd.chars().any(capture::invisible) {
        return Err("working directory contains hidden controls");
    }
    let directory = workspace
        .directory(cwd, budget)
        .map_err(|_| "starting directory unavailable or outside the workspace")?;
    Ok(Proposal {
        preview: Preview {
            command: command.into(),
            cwd: cwd.into(),
            shell: shell::NAME,
            timeout_seconds,
        },
        directory,
    })
}

/// Consumes the proposal after the controller's one-use decision. The callback is
/// bounded preview output, not a raw terminal byte stream. No detached readers.
pub(crate) fn run(
    proposal: Proposal,
    workspace: &Workspace,
    budget: &Budget<'_>,
    output: &mut dyn FnMut(Channel, &str) -> ControlFlow<()>,
) -> io::Result<Outcome> {
    workspace
        .validate_directory(&proposal.directory, budget)
        .map_err(|_| {
            io::Error::other("starting directory changed or command cancelled before launch")
        })?;
    let started = Instant::now();
    let deadline = budget
        .deadline
        .min(started + Duration::from_secs(proposal.preview.timeout_seconds));
    let mut child = platform::Process::spawn(&proposal.preview.command, &proposal.directory)?;
    let mut captures = [capture::Capture::default(), capture::Capture::default()];
    let mut bytes = 0u64;
    let mut displayed = 0usize;
    let mut stop = Stop::Exited;
    let mut buffer = [0u8; 4096];
    'running: loop {
        if budget.cancelled.load(std::sync::atomic::Ordering::Acquire) {
            stop = Stop::Cancelled;
            break;
        }
        if Instant::now() >= deadline {
            stop = Stop::Timeout;
            break;
        }
        let mut received = false;
        for (index, channel) in [Channel::Stdout, Channel::Stderr].into_iter().enumerate() {
            match child.read(channel, &mut buffer) {
                Ok(count) => {
                    received |= count != 0;
                    bytes += count as u64;
                    let text = captures[index].push(&buffer[..count], false);
                    if emit(channel, &text, &mut displayed, output).is_break() {
                        stop = if Instant::now() >= deadline {
                            Stop::Timeout
                        } else {
                            Stop::Cancelled
                        };
                        break 'running;
                    }
                    if bytes > 1024 * 1024 {
                        stop = Stop::OutputLimit;
                        break 'running;
                    }
                }
                Err(_) => {
                    stop = Stop::IoError;
                    break 'running;
                }
            }
        }
        match child.exited() {
            Ok(true) => break,
            Ok(false) => {
                if !received {
                    thread::sleep(Duration::from_millis(8));
                }
            }
            Err(_) => {
                stop = Stop::IoError;
                break;
            }
        }
    }
    // Kill remaining ordinary descendants even if the shell itself already exited.
    // The Linux leader is kept waitable until group termination to prevent PID reuse.
    let cleanup = child.finish();
    // Drain only what is already buffered, bounded independently of descendant behavior.
    for (index, channel) in [Channel::Stdout, Channel::Stderr].into_iter().enumerate() {
        for _ in 0..256 {
            match child.read(channel, &mut buffer) {
                Ok(0) => break,
                Err(_) => {
                    if stop == Stop::Exited {
                        stop = Stop::IoError;
                    }
                    break;
                }
                Ok(count) => {
                    bytes += count as u64;
                    let text = captures[index].push(&buffer[..count], false);
                    let _ = emit(channel, &text, &mut displayed, output);
                    if bytes > 1024 * 1024 {
                        break;
                    }
                }
            }
        }
        let text = captures[index].push(&[], true);
        let _ = emit(channel, &text, &mut displayed, output);
    }
    let [stdout, stderr] = captures;
    Ok(Outcome {
        stop,
        exit: cleanup.as_ref().ok().copied(),
        cleanup_failed: cleanup.is_err(),
        stdout: stdout.tail,
        stderr: stderr.tail,
        truncated: stdout.truncated || stderr.truncated || displayed >= 32768,
        bytes,
        elapsed_ms: started.elapsed().as_millis().try_into().unwrap_or(u64::MAX),
    })
}
fn emit(
    channel: Channel,
    text: &str,
    displayed: &mut usize,
    output: &mut dyn FnMut(Channel, &str) -> ControlFlow<()>,
) -> ControlFlow<()> {
    let mut end = text.len().min(32768usize.saturating_sub(*displayed));
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    *displayed += end;
    if end == 0 {
        ControlFlow::Continue(())
    } else {
        output(channel, &text[..end])
    }
}
