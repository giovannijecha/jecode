use super::RunError;
use crate::cancel::Cancellation;
use crate::sandbox::MARKER;
use std::ffi::{OsStr, OsString};
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::os::windows::process::CommandExt;
use std::process::{Child, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

pub(crate) const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const STOP_DEADLINE: Duration = Duration::from_secs(5);
const HANDSHAKE_STOPPED: &str = "Windows process supervisor stopped before the command started";
const COMPLETION_MISSING: &str =
    "Windows process supervisor ended without completion status; command outcome is unknown";
static NEXT_PIPE: AtomicU64 = AtomicU64::new(0);
#[cfg(test)]
#[path = "windows/sandbox_tests.rs"]
mod sandbox_tests;

/// The supervisor's completion report, delivered once by its reader thread.
pub(super) type Report = mpsc::Receiver<Result<i32, String>>;

pub(super) struct Spawned {
    pub child: Child,
    pub stdout: ChildStdout,
    pub writer: JoinHandle<io::Result<()>>,
    /// The supervisor's completion report; None for a directly started leaf.
    pub completion: Option<Report>,
    pub control: Option<File>,
    pub target_pid: u32,
}

enum Handshake {
    Stage(u8),
    Done(ChildStdout, Result<u32, String>),
}

pub(super) fn spawn(
    command: &Command,
    input: Option<Vec<u8>>,
    cancellation: &Cancellation,
) -> Result<Spawned, RunError> {
    // The supervisor serializes argv instead of calling Command::spawn on
    // the target. Preserve the native API's rejection of embedded NULs.
    let contains_nul = |value: &OsStr| value.encode_wide().any(|unit| unit == 0);
    if contains_nul(command.get_program()) || command.get_args().any(contains_nul) {
        return Err(RunError {
            started: false,
            message: "Executable and arguments must not contain NUL characters".into(),
        });
    }
    if cancellation.requested() {
        return Err(RunError {
            started: false,
            message: "Operation cancelled".into(),
        });
    }
    let executable = crate::native::helper("supervisor").map_err(|message| RunError {
        started: false,
        message,
    })?;
    let mut helper = Command::new(executable);
    configure(&mut helper, command);
    let mut child = helper.spawn().map_err(|error| RunError {
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
    let (sender, receiver) = mpsc::channel();
    let handshake = thread::spawn(move || {
        let result = read_handshake(&mut stdout, |stage| {
            let _ = sender.send(Handshake::Stage(stage));
        });
        let _ = sender.send(Handshake::Done(stdout, result));
    });
    let started = Instant::now();
    let mut last_stage = 0;
    loop {
        match receiver.recv_timeout(Duration::from_millis(20)) {
            Ok(Handshake::Stage(stage)) => last_stage = stage,
            Ok(Handshake::Done(stdout, Ok(target_pid))) => {
                let _ = handshake.join();
                // The supervisor creates both channels before its handshake.
                let mut completion_file =
                    connect(&mut child, &pipe_name, "completion", false, cancellation)?;
                let control = connect(
                    &mut child,
                    &format!("{pipe_name}-stop"),
                    "stop",
                    true,
                    cancellation,
                )?;
                let (report, completion) = mpsc::channel();
                thread::spawn(move || {
                    let _ = report.send(read_completion(&mut completion_file));
                });
                return Ok(Spawned {
                    child,
                    stdout,
                    writer,
                    completion: Some(completion),
                    control: Some(control),
                    target_pid,
                });
            }
            Ok(Handshake::Done(_, Err(message))) => {
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
                if started.elapsed() < Duration::from_secs(15)
                    && !cancellation.requested()
                    && matches!(child.try_wait(), Ok(None)) => {}
            Err(_) => {
                let exited = child.try_wait().ok().flatten();
                let _ = child.kill();
                let _ = child.wait();
                let _ = handshake.join();
                let _ = writer.join();
                // A supervisor that exited on its own may have reported why
                // after the last receive; prefer that over a generic message.
                let mut reported = None;
                for event in receiver.try_iter() {
                    match event {
                        Handshake::Stage(stage) => last_stage = stage,
                        Handshake::Done(_, Err(message)) if message != HANDSHAKE_STOPPED => {
                            reported = Some(message);
                        }
                        Handshake::Done(..) => {}
                    }
                }
                return Err(RunError {
                    started: false,
                    message: if cancellation.requested() {
                        "Operation cancelled".into()
                    } else if let Some(message) = reported {
                        message
                    } else {
                        format!(
                            "Windows process supervisor did not initialize (last stage: {}; exit code: {})",
                            stage_name(last_stage),
                            exited.and_then(|status| status.code()).map_or_else(
                                || "still running".to_string(),
                                |code| code.to_string()
                            )
                        )
                    },
                });
            }
        }
    }
}

/// Starts a leaf executable directly, outside the Job Object supervisor. A
/// leaf starts no processes of its own, so terminating it ends all its work,
/// and it skips the supervisor's .NET start-up (about 100 ms per launch).
pub(super) fn spawn_leaf(command: &Command, input: Option<Vec<u8>>) -> Result<Spawned, RunError> {
    let mut leaf = Command::new(command.get_program());
    leaf.args(command.get_args());
    configure(&mut leaf, command);
    let mut child = leaf.spawn().map_err(|error| RunError {
        started: false,
        message: format!(
            "Could not start {}: {error}",
            command.get_program().to_string_lossy()
        ),
    })?;
    let mut stdin = child.stdin.take().unwrap();
    let writer = thread::spawn(move || {
        if let Some(input) = input {
            stdin.write_all(&input)?;
        }
        Ok(())
    });
    let stdout = child.stdout.take().unwrap();
    Ok(Spawned {
        target_pid: child.id(),
        child,
        stdout,
        writer,
        completion: None,
        control: None,
    })
}

/// The sandbox policy carried by the trusted marker. An empty marker carries
/// no policy and is treated exactly like an absent one, so a command is
/// prepared as confined only when the supervisor will also confine it.
pub(crate) fn sandbox_policy(command: &Command) -> Option<&OsStr> {
    command
        .get_envs()
        .find(|(key, _)| key.to_string_lossy().eq_ignore_ascii_case(MARKER))
        .and_then(|(_, value)| value)
        .filter(|value| !value.is_empty())
}

fn configure(helper: &mut Command, command: &Command) {
    if sandbox_policy(command).is_some() {
        // A confined child inherits the helper's environment. Start it only
        // with the environment selected for this command by the Rust policy.
        helper.env_clear();
    }
    helper
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .creation_flags(CREATE_NO_WINDOW);
    if let Some(directory) = command.get_current_dir() {
        helper.current_dir(ordinary_path(directory));
    }
    crate::auth::isolate_environment(helper);
    for (key, value) in command.get_envs() {
        if key.to_string_lossy().eq_ignore_ascii_case(MARKER) {
            continue;
        }
        if let Some(value) = value {
            helper.env(key, value);
        } else {
            helper.env_remove(key);
        }
    }
    helper.env_remove(MARKER);
}

/// Strips a `\\?\` or `\\?\UNC\` prefix, which child working directories
/// such as cmd.exe's do not accept.
pub(crate) fn ordinary_path(path: &std::path::Path) -> std::path::PathBuf {
    let wide: Vec<u16> = path.as_os_str().encode_wide().collect();
    let verbatim = [b'\\' as u16, b'\\' as u16, b'?' as u16, b'\\' as u16];
    if !wide.starts_with(&verbatim) {
        return path.to_path_buf();
    }
    let unc = [b'U' as u16, b'N' as u16, b'C' as u16, b'\\' as u16];
    let ordinary = if wide[4..].starts_with(&unc) {
        [vec![b'\\' as u16, b'\\' as u16], wide[8..].to_vec()].concat()
    } else {
        wide[4..].to_vec()
    };
    OsString::from_wide(&ordinary).into()
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
    write_string(
        &mut data,
        sandbox_policy(command).unwrap_or_else(|| OsStr::new("")),
    );
    data
}

/// Opens one of the supervisor's private channels. They exist before the
/// handshake completes, so the first attempt normally succeeds; retries back
/// off from 1 ms for a supervisor that is still starting them.
fn connect(
    child: &mut Child,
    name: &str,
    channel: &str,
    write: bool,
    cancellation: &Cancellation,
) -> Result<File, RunError> {
    let path = format!(r"\\.\pipe\{name}");
    let started = Instant::now();
    let mut delay = Duration::from_millis(1);
    loop {
        if let Ok(file) = OpenOptions::new().read(!write).write(write).open(&path) {
            return Ok(file);
        }
        if cancellation.requested() || started.elapsed() >= STOP_DEADLINE {
            let _ = child.kill();
            let _ = child.wait();
            return Err(RunError {
                started: true,
                message: if cancellation.requested() {
                    "Operation cancelled while starting process".into()
                } else {
                    format!(
                        "Windows process supervisor did not open its {channel} channel; command outcome is unknown"
                    )
                },
            });
        }
        if matches!(child.try_wait(), Ok(Some(_))) {
            return Err(RunError {
                started: true,
                message: format!(
                    "Windows process supervisor exited before its {channel} channel opened; command outcome is unknown"
                ),
            });
        }
        thread::sleep(delay);
        delay = (delay * 2).min(Duration::from_millis(10));
    }
}

pub(super) fn completion(report: Report) -> Result<i32, String> {
    report
        .recv()
        .map_err(|_| "Windows process completion worker failed".to_string())?
}

fn read_completion(file: &mut File) -> Result<i32, String> {
    let mut flag = [0];
    file.read_exact(&mut flag)
        .map_err(|_| COMPLETION_MISSING.to_string())?;
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

fn read_handshake(stdout: &mut ChildStdout, mut progress: impl FnMut(u8)) -> Result<u32, String> {
    let mut flag = [0];
    loop {
        stdout
            .read_exact(&mut flag)
            .map_err(|_| HANDSHAKE_STOPPED.to_string())?;
        if flag[0] == 2 {
            let mut stage = [0];
            stdout
                .read_exact(&mut stage)
                .map_err(|_| "Windows process supervisor sent an incomplete stage".to_string())?;
            progress(stage[0]);
            continue;
        }
        break;
    }
    if flag[0] == 1 {
        let mut id = [0; 4];
        stdout.read_exact(&mut id).map_err(|_| {
            "Windows process supervisor sent an incomplete process identity".to_string()
        })?;
        let id = u32::from_le_bytes(id);
        if id == 0 {
            return Err("Windows process supervisor sent an invalid process identity".into());
        }
        return Ok(id);
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

fn stage_name(stage: u8) -> &'static str {
    match stage {
        1 => "managed entry",
        2 => "command channel",
        3 => "sandbox preparation",
        4 => "job and attributes",
        5 => "native child creation",
        6 => "child created",
        _ => "before managed entry",
    }
}

/// Stops the supervised boundary and consumes its completion report.
/// Returns whether the supervisor confirmed that its job has no processes.
///
/// A stop request is acknowledged by the report, which the supervisor sends
/// only after the job is empty. If that fails, killing the supervisor closes
/// the only job handle and its kill-on-close limit ends the job; a killed
/// supervisor cannot report, so its missing report is not an error.
pub(super) fn terminate(
    child: &mut Child,
    control: Option<&File>,
    completion: &mut Option<Report>,
) -> Result<bool, String> {
    let exited = child
        .try_wait()
        .map_err(|error| format!("Could not inspect process supervisor: {error}"))?
        .is_some();
    let report = completion.take();
    let stopped = if exited {
        // An exited supervisor has already written its report or closed it.
        report.map(|report| report.recv_timeout(STOP_DEADLINE))
    } else if let (Some(mut control), Some(report)) = (control, report)
        && control.write_all(&[1]).is_ok()
    {
        Some(report.recv_timeout(STOP_DEADLINE))
    } else {
        None
    };
    match stopped {
        Some(Ok(Ok(_))) => return Ok(true),
        // The supervisor could not confirm that its job is empty.
        Some(Ok(Err(message))) if message != COMPLETION_MISSING => return Err(message),
        _ if exited => return Ok(false),
        _ => {}
    }
    child
        .kill()
        .map_err(|error| format!("Could not stop process supervisor: {error}"))?;
    child
        .wait()
        .map_err(|error| format!("Could not wait for process supervisor: {error}"))?;
    Ok(false)
}

/// Reaps a supervisor after its report and output have ended. It exits right
/// after reporting; one that remains is killed so no supervisor outlives
/// the run.
pub(super) fn reap(child: &mut Child) {
    if matches!(child.try_wait(), Ok(None)) {
        let _ = child.kill();
    }
    let _ = child.wait();
}
