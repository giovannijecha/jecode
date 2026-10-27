//! Windows ConPTY host. The host is launched through `process::start_logged`,
//! so its console child inherits the existing Job Object lifetime.

use crate::cancel::Cancellation;
use std::fs::{File, OpenOptions};
use std::process::Command;
use std::thread;
use std::time::{Duration, Instant};

const SOURCE: &str = include_str!("windows.cs");

/// Prepare the owned ConPTY host for the already validated executable command.
/// The target's working directory and explicit environment changes are applied
/// to the host, which passes them to the terminal child unchanged.
pub(super) fn command(
    target: &Command,
    endpoint: &str,
    cols: u16,
    rows: u16,
    cancellation: &Cancellation,
) -> Result<Command, String> {
    if endpoint.is_empty()
        || !endpoint
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
    {
        return Err("Invalid Windows PTY control endpoint".into());
    }
    if cols == 0 || rows == 0 || cols > i16::MAX as u16 || rows > i16::MAX as u16 {
        return Err("Windows PTY size must be between 1 and 32767 cells".into());
    }
    let executable =
        crate::process::compile_helper("pty-v1", SOURCE, cancellation).ok_or_else(|| {
            if cancellation.requested() {
                "Operation cancelled".to_string()
            } else {
                "Could not compile the Windows PTY bridge".to_string()
            }
        })?;
    let mut host = Command::new(executable);
    host.arg(endpoint)
        .arg(cols.to_string())
        .arg(rows.to_string())
        .arg(target.get_program())
        .args(target.get_args());
    if let Some(directory) = target.get_current_dir() {
        host.current_dir(directory);
    }
    for (key, value) in target.get_envs() {
        if let Some(value) = value {
            host.env(key, value);
        } else {
            host.env_remove(key);
        }
    }
    Ok(host)
}

/// Connect only after `start_logged` has returned. The C# host creates this
/// endpoint after CreateProcessW has successfully started the PTY child.
/// The caller must stop/supervise the launched host if this returns an error.
pub(super) fn connect(endpoint: &str, cancellation: &Cancellation) -> Result<File, String> {
    let path = format!(r"\\.\pipe\{endpoint}");
    let started = Instant::now();
    loop {
        if cancellation.requested() {
            return Err("Operation cancelled while starting Windows PTY".into());
        }
        if let Ok(file) = OpenOptions::new().write(true).open(&path) {
            return Ok(file);
        }
        if started.elapsed() >= Duration::from_secs(5) {
            return Err("Windows PTY control channel did not become ready".into());
        }
        thread::sleep(Duration::from_millis(10));
    }
}
