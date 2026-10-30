use crate::cancel::Cancellation;
#[cfg(test)]
use std::fs;
use std::fs::File;
use std::io;
use std::os::fd::OwnedFd;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

#[cfg(test)]
const SOURCE: &str = include_str!("unix.c");
const START_LIMIT: Duration = Duration::from_secs(5);

/// Wraps the target in the owned native PTY bridge. The caller launches this
/// command with `process::start_logged`, preserving the normal Unix guardian.
pub(super) fn command(
    target: &Command,
    endpoint: &str,
    columns: u16,
    rows: u16,
    cancellation: &Cancellation,
) -> Result<Command, String> {
    if columns == 0 || rows == 0 {
        return Err("PTY dimensions must be positive".into());
    }
    let helper = helper(cancellation)?;
    let mut command = Command::new(helper);
    command
        .arg(endpoint)
        .arg(columns.to_string())
        .arg(rows.to_string())
        .arg(target.get_program())
        .args(target.get_args())
        .stdin(Stdio::null());
    if let Some(directory) = target.get_current_dir() {
        command.current_dir(directory);
    }
    for (key, value) in target.get_envs() {
        if let Some(value) = value {
            command.env(key, value);
        } else {
            command.env_remove(key);
        }
    }
    Ok(command)
}

/// Connects to the bridge's private Unix socket after it has begun listening.
/// The returned File is nonblocking so a full input queue cannot stall UI work.
pub(super) fn connect(endpoint: &str, cancellation: &Cancellation) -> Result<File, String> {
    let started = Instant::now();
    loop {
        if cancellation.requested() {
            return Err("Operation cancelled".into());
        }
        match UnixStream::connect(endpoint) {
            Ok(stream) => {
                stream
                    .set_nonblocking(true)
                    .map_err(|error| format!("Could not configure Unix PTY input: {error}"))?;
                let descriptor: OwnedFd = stream.into();
                return Ok(File::from(descriptor));
            }
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused
                ) && started.elapsed() < START_LIMIT =>
            {
                thread::sleep(Duration::from_millis(10));
            }
            Err(error) => return Err(format!("Could not connect to Unix PTY: {error}")),
        }
    }
}

pub(super) fn connect_control(endpoint: &str, cancellation: &Cancellation) -> Result<File, String> {
    connect(endpoint, cancellation)
}

fn helper(cancellation: &Cancellation) -> Result<PathBuf, String> {
    if cancellation.requested() {
        return Err("Operation cancelled".into());
    }
    crate::native::helper("pty_unix")
}
#[cfg(test)]
#[path = "unix_tests.rs"]
mod tests;
