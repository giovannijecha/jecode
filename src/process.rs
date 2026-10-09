use crate::cancel::Cancellation;
use std::io::Read;
#[cfg(any(unix, test))]
use std::process::Stdio;
use std::process::{Child, Command};
use std::sync::{Arc, Mutex, mpsc};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

#[cfg(unix)]
mod unix;
#[cfg(windows)]
mod windows;

#[derive(Default)]
struct Capture {
    bytes: Vec<u8>,
    truncated: bool,
    error: Option<String>,
    total_bytes: usize,
}

pub struct Output {
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub exit_code: Option<i32>,
    pub timed_out: bool,
    pub truncated: bool,
    pub stdout_bytes: usize,
    pub stderr_bytes: usize,
    pub stdout_truncated: bool,
    pub stderr_truncated: bool,
    pub cancelled: bool,
}

#[derive(Debug)]
pub struct RunError {
    pub started: bool,
    pub message: String,
}
impl From<String> for RunError {
    fn from(message: String) -> Self {
        Self {
            started: true,
            message,
        }
    }
}

pub fn run_observed(
    command: &mut Command,
    input: Option<Vec<u8>>,
    timeout: impl Into<Option<Duration>>,
    output_limit: usize,
    cancellation: &Cancellation,
    idle: Option<Duration>,
    observer: &mut impl FnMut(&[u8]) -> Result<(), String>,
) -> Result<Output, String> {
    run_inner(
        command,
        input,
        timeout.into(),
        cancellation,
        observer,
        CaptureOptions {
            limit: output_limit,
            tail: false,
            spools: [None, None],
            observed: true,
            idle,
        },
    )
    .map_err(|error| error.message)
}

pub fn run_logged(
    command: &mut Command,
    timeout: Option<Duration>,
    output_limit: usize,
    cancellation: &Cancellation,
    spools: [crate::output::Spool; 2],
) -> Result<Output, RunError> {
    let [stdout, stderr] = spools;
    run_inner(
        command,
        None,
        timeout,
        cancellation,
        &mut |_| Ok(()),
        CaptureOptions {
            limit: output_limit,
            tail: true,
            spools: [Some(stdout), Some(stderr)],
            observed: false,
            idle: None,
        },
    )
}

struct CaptureOptions {
    limit: usize,
    tail: bool,
    spools: [Option<crate::output::Spool>; 2],
    observed: bool,
    idle: Option<Duration>,
}

fn run_inner(
    command: &mut Command,
    input: Option<Vec<u8>>,
    timeout: Option<Duration>,
    cancellation: &Cancellation,
    observer: &mut impl FnMut(&[u8]) -> Result<(), String>,
    options: CaptureOptions,
) -> Result<Output, RunError> {
    if cancellation.requested() {
        return Err(RunError {
            started: false,
            message: "Operation cancelled".into(),
        });
    }
    #[cfg(unix)]
    let unix::Spawned {
        mut child,
        stdout: child_stdout,
        writer,
        owner: _owner,
    } = unix::spawn(command, input, cancellation)?;
    #[cfg(windows)]
    let windows::Spawned {
        mut child,
        stdout: child_stdout,
        writer: stdin_writer,
        completion,
    } = windows::spawn(command, input, cancellation)?;
    let started = Instant::now();
    let mut last_activity = started;
    let stdout = Arc::new(Mutex::new(Capture::default()));
    let stderr = Arc::new(Mutex::new(Capture::default()));
    let (sender, chunks) = mpsc::channel();
    let [stdout_spool, stderr_spool] = options.spools;
    let stdout_reader = capture(
        child_stdout,
        Arc::clone(&stdout),
        options.limit,
        options.observed.then_some(sender),
        options.tail,
        stdout_spool,
    );
    let stderr_reader = capture(
        child.stderr.take().unwrap(),
        Arc::clone(&stderr),
        options.limit,
        None,
        options.tail,
        stderr_spool,
    );
    #[cfg(windows)]
    let writer = Some(stdin_writer);
    let mut status = None;
    let mut timed_out = false;
    let mut cancelled = false;
    let mut observer_error = None;
    let mut cleanup_error = None;
    #[cfg(windows)]
    let mut terminated = false;

    loop {
        while let Ok(chunk) = chunks.try_recv() {
            last_activity = Instant::now();
            if observer_error.is_none() {
                observer_error = observer(&chunk).err();
            }
        }
        if observer_error.is_some() {
            #[cfg(windows)]
            {
                terminated = true;
            }
            cleanup_error = terminate(&mut child).err();
            break;
        }
        if stdout.lock().unwrap().error.is_some() || stderr.lock().unwrap().error.is_some() {
            #[cfg(windows)]
            {
                terminated = true;
            }
            cleanup_error = terminate(&mut child).err();
            break;
        }
        if status.is_none() {
            match child.try_wait() {
                Ok(value) => status = value,
                Err(error) => {
                    let cleanup = terminate(&mut child).err();
                    return Err(format!(
                        "Could not wait for process: {error}{}",
                        cleanup.map_or(String::new(), |error| format!("; {error}"))
                    )
                    .into());
                }
            }
        }
        if status.is_some()
            && stdout_reader.is_finished()
            && stderr_reader.is_finished()
            && writer.as_ref().is_none_or(JoinHandle::is_finished)
        {
            break;
        }
        if cancellation.requested()
            || timeout.is_some_and(|timeout| started.elapsed() >= timeout)
            || options
                .idle
                .is_some_and(|idle| last_activity.elapsed() >= idle)
        {
            cancelled = cancellation.requested();
            timed_out = !cancelled;
            #[cfg(windows)]
            {
                terminated = true;
            }
            cleanup_error = terminate(&mut child).err();
            #[cfg(unix)]
            {
                status = child.try_wait().ok().flatten().or(status);
            }
            // An inherited pipe must not make an already timed-out command block forever.
            let drain_started = Instant::now();
            while !(stdout_reader.is_finished() && stderr_reader.is_finished())
                && drain_started.elapsed() < Duration::from_millis(250)
            {
                thread::sleep(Duration::from_millis(5));
            }
            if !stdout_reader.is_finished() || !stderr_reader.is_finished() {
                cleanup_error.get_or_insert_with(|| {
                    "Process cleanup could not be confirmed: output capture remains active".into()
                });
            }
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }

    while let Ok(chunk) = chunks.try_recv() {
        if observer_error.is_none() {
            observer_error = observer(&chunk).err();
        }
    }
    if let Some(error) = observer_error {
        return Err(format!(
            "{error}{}",
            cleanup_error.map_or(String::new(), |cleanup| format!("; {cleanup}"))
        )
        .into());
    }
    if let Some(error) = cleanup_error {
        let message = if cancelled {
            format!("Operation cancelled; {error}")
        } else if timed_out {
            format!("Process timed out; {error}")
        } else {
            error
        };
        return Err(message.into());
    }

    #[cfg(windows)]
    let native_exit_code = if !terminated {
        Some(windows::completion(completion).map_err(RunError::from)?)
    } else {
        None
    };

    if let Some(writer) = writer
        && writer.is_finished()
        && !timed_out
        && !cancelled
    {
        writer
            .join()
            .map_err(|_| "Process input worker failed".to_string())?
            .map_err(|error| format!("Could not send process input: {error}"))?;
    }
    let mut stdout = stdout.lock().unwrap();
    let mut stderr = stderr.lock().unwrap();
    if let Some(error) = stdout.error.as_ref().or(stderr.error.as_ref()) {
        return Err(format!("Could not read process output: {error}").into());
    }
    Ok(Output {
        stdout: std::mem::take(&mut stdout.bytes),
        stderr: std::mem::take(&mut stderr.bytes),
        exit_code: {
            #[cfg(windows)]
            {
                native_exit_code
            }
            #[cfg(unix)]
            {
                status.and_then(|status| status.code())
            }
        },
        timed_out,
        truncated: stdout.truncated || stderr.truncated,
        stdout_bytes: stdout.total_bytes,
        stderr_bytes: stderr.total_bytes,
        stdout_truncated: stdout.truncated,
        stderr_truncated: stderr.truncated,
        cancelled,
    })
}

fn capture(
    mut reader: impl Read + Send + 'static,
    output: Arc<Mutex<Capture>>,
    limit: usize,
    chunks: Option<mpsc::Sender<Vec<u8>>>,
    tail: bool,
    mut spool: Option<crate::output::Spool>,
) -> JoinHandle<()> {
    thread::spawn(move || {
        let mut buffer = [0; 8192];
        loop {
            match reader.read(&mut buffer) {
                Ok(0) => {
                    if let Some(spool) = &mut spool
                        && let Err(error) = spool.finish()
                    {
                        output.lock().unwrap().error = Some(error.to_string());
                    }
                    break;
                }
                Ok(length) => {
                    if let Some(spool) = &mut spool
                        && let Err(error) = spool.write(&buffer[..length])
                    {
                        output.lock().unwrap().error = Some(error.to_string());
                        break;
                    }
                    let mut output = output.lock().unwrap();
                    output.total_bytes = output.total_bytes.saturating_add(length);
                    let keep = length.min(limit.saturating_sub(output.bytes.len()));
                    if tail {
                        output.bytes.extend_from_slice(&buffer[..length]);
                        let drop = output.bytes.len().saturating_sub(limit);
                        output.bytes.drain(..drop);
                    } else {
                        output.bytes.extend_from_slice(&buffer[..keep]);
                    }
                    output.truncated |= keep < length;
                    if keep > 0
                        && let Some(chunks) = &chunks
                    {
                        let _ = chunks.send(buffer[..keep].to_vec());
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(error) => {
                    output.lock().unwrap().error = Some(error.to_string());
                    break;
                }
            }
        }
    })
}

#[cfg(windows)]
fn terminate(child: &mut Child) -> Result<(), String> {
    if child
        .try_wait()
        .map_err(|error| format!("Could not inspect process supervisor: {error}"))?
        .is_some()
    {
        return Ok(());
    }
    child
        .kill()
        .map_err(|error| format!("Could not stop process supervisor: {error}"))?;
    child
        .wait()
        .map_err(|error| format!("Could not wait for process supervisor: {error}"))?;
    Ok(())
}

#[cfg(unix)]
fn terminate(child: &mut Child) -> Result<(), String> {
    let mut cleanup = Command::new("kill");
    cleanup
        .args(["-KILL", "--", &format!("-{}", child.id())])
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    let result = (|| {
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
