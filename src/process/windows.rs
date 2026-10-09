use super::RunError;
use crate::cancel::Cancellation;
use std::ffi::OsStr;
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::windows::ffi::OsStrExt;
use std::os::windows::process::CommandExt;
use std::process::{Child, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

const CREATE_NO_WINDOW: u32 = 0x0800_0000;
static NEXT_PIPE: AtomicU64 = AtomicU64::new(0);
mod cache;

pub(super) struct Spawned {
    pub child: Child,
    pub stdout: ChildStdout,
    pub writer: JoinHandle<io::Result<()>>,
    pub completion: JoinHandle<Result<i32, String>>,
}

pub(super) fn spawn(
    command: &Command,
    input: Option<Vec<u8>>,
    cancellation: &Cancellation,
) -> Result<Spawned, RunError> {
    let executable = cache::supervisor(cancellation);
    if cancellation.requested() {
        return Err(RunError {
            started: false,
            message: "Operation cancelled".into(),
        });
    }
    let mut helper = match &executable {
        Some(path) => Command::new(path),
        None => dynamic_supervisor(),
    };
    configure(&mut helper, command);
    let mut child = helper
        .spawn()
        .or_else(|error| {
            if executable.is_none() {
                return Err(error);
            }
            if cancellation.requested() {
                return Err(io::Error::new(
                    io::ErrorKind::Interrupted,
                    "Operation cancelled",
                ));
            }
            // If a cached executable was removed or blocked, retain the original
            // startup path without changing whether the target has run.
            let mut fallback = dynamic_supervisor();
            configure(&mut fallback, command);
            fallback.spawn()
        })
        .map_err(|error| RunError {
            started: false,
            message: if error.kind() == io::ErrorKind::Interrupted {
                "Operation cancelled".into()
            } else {
                format!("Could not start Windows process supervisor: {error}")
            },
        })?;
    let pipe_name = format!(
        "jecode-process-{}-{}",
        std::process::id(),
        NEXT_PIPE.fetch_add(1, Ordering::Relaxed)
    );
    let config = encode_command(command, &pipe_name);
    let mut stdin = child.stdin.take().unwrap();
    let writer = thread::spawn(move || {
        stdin.write_all(&config)?;
        if let Some(input) = input {
            stdin.write_all(&input)?;
        }
        Ok(())
    });
    let mut stdout = child.stdout.take().unwrap();
    let (sender, receiver) = mpsc::sync_channel(1);
    let handshake = thread::spawn(move || {
        let result = read_handshake(&mut stdout);
        let _ = sender.send((stdout, result));
    });
    let started = Instant::now();
    loop {
        match receiver.recv_timeout(Duration::from_millis(20)) {
            Ok((stdout, Ok(()))) => {
                let _ = handshake.join();
                let mut completion_file = connect_completion(&mut child, &pipe_name, cancellation)?;
                let completion = thread::spawn(move || read_completion(&mut completion_file));
                return Ok(Spawned {
                    child,
                    stdout,
                    writer,
                    completion,
                });
            }
            Ok((_, Err(message))) => {
                let _ = handshake.join();
                let _ = child.kill();
                let _ = child.wait();
                let _ = writer.join();
                return Err(RunError {
                    started: false,
                    message,
                });
            }
            Err(mpsc::RecvTimeoutError::Timeout)
                if started.elapsed() < Duration::from_secs(15) && !cancellation.requested() => {}
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = handshake.join();
                let _ = writer.join();
                return Err(RunError {
                    started: false,
                    message: if cancellation.requested() {
                        "Operation cancelled".into()
                    } else {
                        "Windows process supervisor did not initialize".into()
                    },
                });
            }
        }
    }
}

fn dynamic_supervisor() -> Command {
    let script =
        include_str!("windows.ps1").replace("__JECODE_NATIVE_SOURCE__", include_str!("windows.cs"));
    let mut helper = Command::new("powershell.exe");
    helper.args([
        "-NoLogo",
        "-NoProfile",
        "-NonInteractive",
        "-Command",
        &script,
    ]);
    helper
}

fn configure(helper: &mut Command, command: &Command) {
    helper
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .creation_flags(CREATE_NO_WINDOW)
        .env_remove("OPENROUTER_API_KEY");
    if let Some(directory) = command.get_current_dir() {
        helper.current_dir(directory);
    }
    for (key, value) in command.get_envs() {
        if let Some(value) = value {
            helper.env(key, value);
        } else {
            helper.env_remove(key);
        }
    }
}

fn encode_command(command: &Command, pipe_name: &str) -> Vec<u8> {
    let mut data = Vec::new();
    data.extend_from_slice(&std::process::id().to_le_bytes());
    write_string(&mut data, OsStr::new(pipe_name));
    write_string(&mut data, command.get_program());
    data.extend_from_slice(&(command.get_args().len() as u32).to_le_bytes());
    for argument in command.get_args() {
        write_string(&mut data, argument);
    }
    data
}

fn connect_completion(
    child: &mut Child,
    name: &str,
    cancellation: &Cancellation,
) -> Result<File, RunError> {
    let path = format!(r"\\.\pipe\{name}");
    let started = Instant::now();
    loop {
        if let Ok(file) = OpenOptions::new().read(true).open(&path) {
            return Ok(file);
        }
        if cancellation.requested() || started.elapsed() >= Duration::from_secs(5) {
            let _ = child.kill();
            let _ = child.wait();
            return Err(RunError {
                started: true,
                message: if cancellation.requested() {
                    "Operation cancelled while starting process".into()
                } else {
                    "Windows process supervisor did not open its completion channel; command outcome is unknown".into()
                },
            });
        }
        if matches!(child.try_wait(), Ok(Some(_))) {
            return Err(RunError {
                started: true,
                message: "Windows process supervisor exited before completion channel opened; command outcome is unknown".into(),
            });
        }
        thread::sleep(Duration::from_millis(10));
    }
}

pub(super) fn completion(reader: JoinHandle<Result<i32, String>>) -> Result<i32, String> {
    reader
        .join()
        .map_err(|_| "Windows process completion worker failed".to_string())?
}

fn read_completion(file: &mut File) -> Result<i32, String> {
    let mut flag = [0];
    file.read_exact(&mut flag).map_err(|_| {
        "Windows process supervisor ended without completion status; command outcome is unknown"
            .to_string()
    })?;
    let mut bytes = [0; 4];
    file.read_exact(&mut bytes)
        .map_err(|_| "Windows process supervisor sent incomplete completion status".to_string())?;
    if flag[0] == 1 {
        return Ok(i32::from_le_bytes(bytes));
    }
    if flag[0] != 0 {
        return Err("Windows process supervisor sent invalid completion status".into());
    }
    let length = u32::from_le_bytes(bytes) as usize;
    if length > 4096 {
        return Err("Windows process supervisor sent oversized failure status".into());
    }
    let mut message = vec![0; length];
    file.read_exact(&mut message)
        .map_err(|_| "Windows process supervisor sent incomplete failure status".to_string())?;
    Err(format!(
        "Windows process supervisor failed after command start; outcome is unknown: {}",
        String::from_utf8_lossy(&message)
    ))
}

fn write_string(data: &mut Vec<u8>, value: &OsStr) {
    let units: Vec<_> = value.encode_wide().collect();
    data.extend_from_slice(&(units.len() as u32).to_le_bytes());
    for unit in units {
        data.extend_from_slice(&unit.to_le_bytes());
    }
}

fn read_handshake(stdout: &mut ChildStdout) -> Result<(), String> {
    let mut flag = [0];
    stdout
        .read_exact(&mut flag)
        .map_err(|_| "Windows process supervisor stopped before the command started".to_string())?;
    if flag[0] == 1 {
        return Ok(());
    }
    if flag[0] != 0 {
        return Err("Windows process supervisor sent an invalid response".into());
    }
    let mut length = [0; 4];
    stdout
        .read_exact(&mut length)
        .map_err(|_| "Windows process supervisor sent an incomplete error".to_string())?;
    let length = u32::from_le_bytes(length) as usize;
    if length > 4096 {
        return Err("Windows process supervisor sent an oversized error".into());
    }
    let mut bytes = vec![0; length];
    stdout
        .read_exact(&mut bytes)
        .map_err(|_| "Windows process supervisor sent an incomplete error".to_string())?;
    Err(format!(
        "Could not start process: {}",
        String::from_utf8_lossy(&bytes)
    ))
}
