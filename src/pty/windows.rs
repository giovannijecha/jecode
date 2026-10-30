//! Windows ConPTY host. The host is launched through `process::start_logged`,
//! so its console child inherits the existing Job Object lifetime.

use crate::cancel::Cancellation;
use crate::sandbox::MARKER;
use std::fs::{File, OpenOptions};
use std::os::windows::process::CommandExt;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

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
    if crate::process::sandbox_policy(target).is_some() {
        return Err("Workspace interactive terminals cannot attach to Windows ConPTY under process security confinement; use Full for tty or run exec without tty".into());
    }
    if cancellation.requested() {
        return Err("Operation cancelled".into());
    }
    let executable = crate::native::helper("pty")?;
    let mut host = Command::new(executable);
    host.arg(endpoint)
        .arg(cols.to_string())
        .arg(rows.to_string())
        .arg(target.get_program())
        .args(target.get_args());
    if let Some(directory) = target.get_current_dir() {
        host.current_dir(crate::process::ordinary_path(directory));
    }
    for (key, value) in target.get_envs() {
        if key.to_string_lossy().eq_ignore_ascii_case(MARKER) {
            continue;
        }
        if let Some(value) = value {
            host.env(key, value);
        } else {
            host.env_remove(key);
        }
    }
    host.env_remove(MARKER);
    Ok(host)
}

/// Connect only after `start_logged` has returned. The C# host creates this
/// endpoint after CreateProcessW has successfully started the PTY child.
/// The caller must stop/supervise the launched host if this returns an error.
pub(super) fn connect(
    endpoint: &str,
    expected_pid: u32,
    cancellation: &Cancellation,
) -> Result<File, String> {
    connect_pipe(endpoint, expected_pid, cancellation)
}

pub(super) fn connect_control(
    endpoint: &str,
    expected_pid: u32,
    cancellation: &Cancellation,
) -> Result<File, String> {
    connect_pipe(&format!("{endpoint}-control"), expected_pid, cancellation)
}

fn connect_pipe(
    name: &str,
    expected_pid: u32,
    cancellation: &Cancellation,
) -> Result<File, String> {
    let path = format!(r"\\.\pipe\{name}");
    let started = Instant::now();
    loop {
        if cancellation.requested() {
            return Err("Operation cancelled while starting Windows PTY".into());
        }
        if let Ok(file) = OpenOptions::new().write(true).open(&path) {
            verify_server(&file, expected_pid, cancellation)?;
            return Ok(file);
        }
        if started.elapsed() >= Duration::from_secs(5) {
            return Err("Windows PTY control channel did not become ready".into());
        }
        thread::sleep(Duration::from_millis(10));
    }
}

fn verify_server(
    file: &File,
    expected_pid: u32,
    cancellation: &Cancellation,
) -> Result<(), String> {
    let executable = crate::native::helper("pty")?;
    let mut verifier = Command::new(executable);
    verifier
        .arg("--verify-pipe")
        .arg(expected_pid.to_string())
        .stdin(Stdio::from(
            file.try_clone()
                .map_err(|e| format!("Could not verify terminal pipe: {e}"))?,
        ))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .creation_flags(crate::process::CREATE_NO_WINDOW);
    crate::auth::isolate_environment(&mut verifier);
    let mut child = verifier
        .spawn()
        .map_err(|e| format!("Could not start terminal identity verifier: {e}"))?;
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => return Ok(()),
            Ok(Some(_)) => return Err("Windows PTY endpoint belongs to another process".into()),
            Ok(None) if !cancellation.requested() && started.elapsed() < Duration::from_secs(5) => {
                thread::sleep(Duration::from_millis(10));
            }
            Ok(None) | Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(if cancellation.requested() {
                    "Operation cancelled while verifying Windows PTY".into()
                } else {
                    "Could not verify Windows PTY endpoint identity".into()
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader};

    #[test]
    fn host_gets_an_ordinary_directory_and_no_empty_marker() {
        let mut target = Command::new("cmd.exe");
        target.current_dir(r"\\?\C:\Windows").env(MARKER, "");
        let host = command(&target, "endpoint-1", 80, 24, &Cancellation::default()).unwrap();
        assert_eq!(
            host.get_current_dir(),
            Some(std::path::Path::new(r"C:\Windows"))
        );
        assert!(host.get_envs().all(|(key, value)| {
            !key.to_string_lossy().eq_ignore_ascii_case(MARKER) || value.is_none()
        }));
    }

    #[test]
    fn rejects_pipe_served_by_unexpected_process() {
        let name = format!("jecode-pty-spoof-{}", crate::sessions::identifier());
        let script = format!(
            "$p = New-Object System.IO.Pipes.NamedPipeServerStream('{name}', 'In', 1, 'Byte', 'None'); Write-Output READY; try {{ $p.WaitForConnection(); $b = New-Object byte[] 1; $read = $p.BeginRead($b, 0, 1, $null, $null); if ($read.AsyncWaitHandle.WaitOne(2000)) {{ Write-Output ('RECEIVED:' + $p.EndRead($read)) }} else {{ Write-Output TIMEOUT }} }} finally {{ $p.Dispose() }}"
        );
        let mut server = Command::new("powershell.exe")
            .args(["-NoProfile", "-NonInteractive", "-Command", &script])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let mut output = BufReader::new(server.stdout.take().unwrap());
        let mut ready = String::new();
        output.read_line(&mut ready).unwrap();
        assert_eq!(ready.trim(), "READY");
        let result = connect(&name, std::process::id(), &Cancellation::default());
        assert_eq!(
            result.err().as_deref(),
            Some("Windows PTY endpoint belongs to another process")
        );
        let mut received = String::new();
        output.read_line(&mut received).unwrap();
        assert_eq!(received.trim(), "RECEIVED:0");
        assert!(server.wait().unwrap().success());
    }

    #[test]
    fn bridge_failure_and_target_exit_one_have_distinct_codes() {
        let executable = crate::native::helper("pty").unwrap();
        let missing = Command::new(&executable)
            .args([
                crate::sessions::identifier().as_str(),
                "80",
                "24",
                r"C:\jecode-missing-target.exe",
            ])
            .output()
            .unwrap();
        assert_eq!(missing.status.code(), Some(125));

        let name = format!("jecode-pty-exit-{}", crate::sessions::identifier());
        let child = Command::new(&executable)
            .args([&name, "80", "24", "cmd.exe", "/c", "exit", "1"])
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let input = connect(&name, child.id(), &Cancellation::default()).unwrap();
        let control = connect_control(&name, child.id(), &Cancellation::default()).unwrap();
        drop(input);
        drop(control);
        let output = child.wait_with_output().unwrap();
        assert_eq!(
            output.status.code(),
            Some(1),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
