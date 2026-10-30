use crate::cancel::Cancellation;
#[cfg(all(windows, test))]
use std::process::Stdio;
use std::process::{Child, Command};
use std::sync::{Arc, Mutex, mpsc};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

#[cfg(unix)]
mod unix;
#[cfg(windows)]
mod windows;

mod capture;
use capture::{Capture, Events, capture};
#[cfg(windows)]
pub(crate) use windows::{CREATE_NO_WINDOW, ordinary_path, sandbox_policy};

pub struct Output {
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub exit_code: Option<i32>,
    /// Input delivery failed after launch; the child's output and exit status remain available.
    pub input_error: Option<String>,
    pub timed_out: bool,
    pub stdout_bytes: usize,
    pub stderr_bytes: usize,
    pub stdout_truncated: bool,
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

/// Observe complete stdout while bounding the bytes retained in `Output`.
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
            leaf: false,
        },
    )
    .map_err(|error| error.message)
}

/// Like `run_observed`, for an executable that starts no child processes,
/// such as an owned HTTP adapter. Windows launches it directly instead of through the Job
/// Object supervisor; Unix keeps its process-group boundary.
pub fn run_leaf(
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
            leaf: true,
        },
    )
    .map_err(|error| error.message)
}

#[cfg(test)]
pub fn run_logged(
    command: &mut Command,
    timeout: Option<Duration>,
    output_limit: usize,
    cancellation: &Cancellation,
    spools: [crate::output::Spool; 2],
) -> Result<Output, RunError> {
    run_logged_input(command, None, timeout, output_limit, cancellation, spools)
}

/// Runs a supervised command while saving both streams and sending optional stdin.
pub fn run_logged_input(
    command: &mut Command,
    input: Option<Vec<u8>>,
    timeout: Option<Duration>,
    output_limit: usize,
    cancellation: &Cancellation,
    spools: [crate::output::Spool; 2],
) -> Result<Output, RunError> {
    let [stdout, stderr] = spools;
    run_inner(
        command,
        input,
        timeout,
        cancellation,
        &mut |_| Ok(()),
        CaptureOptions {
            limit: output_limit,
            tail: true,
            spools: [Some(stdout), Some(stderr)],
            observed: false,
            idle: None,
            leaf: false,
        },
    )
}

struct CaptureOptions {
    limit: usize,
    tail: bool,
    spools: [Option<crate::output::Spool>; 2],
    observed: bool,
    idle: Option<Duration>,
    /// Starts a childless executable directly on Windows.
    leaf: bool,
}

fn run_inner(
    command: &mut Command,
    input: Option<Vec<u8>>,
    timeout: Option<Duration>,
    cancellation: &Cancellation,
    observer: &mut impl FnMut(&[u8]) -> Result<(), String>,
    options: CaptureOptions,
) -> Result<Output, RunError> {
    start(command, input, cancellation, options)?.supervise(timeout, cancellation, observer)
}

/// Starts a logged command without waiting for it. `cancellation` applies only
/// while the supervised boundary launches; `Started::supervise` receives the
/// cancellation that owns the running process.
pub fn start_logged(
    command: &mut Command,
    cancellation: &Cancellation,
    spools: [crate::output::Spool; 2],
) -> Result<Started, RunError> {
    start_logged_input(command, None, cancellation, spools)
}

/// Starts a logged command with optional stdin for conversation-owned processes.
pub fn start_logged_input(
    command: &mut Command,
    input: Option<Vec<u8>>,
    cancellation: &Cancellation,
    spools: [crate::output::Spool; 2],
) -> Result<Started, RunError> {
    let [stdout, stderr] = spools;
    start(
        command,
        input,
        cancellation,
        CaptureOptions {
            // Output lives in the spools; nothing is kept in memory.
            limit: 0,
            tail: true,
            spools: [Some(stdout), Some(stderr)],
            observed: false,
            idle: None,
            leaf: false,
        },
    )
}

/// A launched process inside its supervised boundary.
pub struct Started {
    child: Child,
    stdout: Arc<Mutex<Capture>>,
    stderr: Arc<Mutex<Capture>>,
    events: Events,
    writer: Option<JoinHandle<std::io::Result<()>>>,
    #[cfg(windows)]
    completion: Option<windows::Report>,
    #[cfg(windows)]
    control: Option<std::fs::File>,
    #[cfg(windows)]
    target_pid: u32,
    #[cfg(unix)]
    owner: unix::Owner,
    started: Instant,
    idle: Option<Duration>,
}

fn start(
    command: &mut Command,
    input: Option<Vec<u8>>,
    cancellation: &Cancellation,
    options: CaptureOptions,
) -> Result<Started, RunError> {
    if cancellation.requested() {
        return Err(RunError {
            started: false,
            message: "Operation cancelled".into(),
        });
    }
    // Unix keeps its process-group boundary for every command.
    #[cfg(unix)]
    let _ = options.leaf;
    #[cfg(unix)]
    let unix::Spawned {
        mut child,
        stdout: child_stdout,
        writer,
        owner,
    } = unix::spawn(command, input, cancellation)?;
    #[cfg(windows)]
    let windows::Spawned {
        mut child,
        stdout: child_stdout,
        writer: stdin_writer,
        completion,
        control,
        target_pid,
    } = if options.leaf {
        windows::spawn_leaf(command, input)?
    } else {
        windows::spawn(command, input, cancellation)?
    };
    let started = Instant::now();
    let stdout = Arc::new(Mutex::new(Capture::default()));
    let stderr = Arc::new(Mutex::new(Capture::default()));
    let (sender, events) = mpsc::channel();
    let [stdout_spool, stderr_spool] = options.spools;
    capture(
        child_stdout,
        Arc::clone(&stdout),
        options.limit,
        sender.clone(),
        options.observed,
        options.tail,
        stdout_spool,
    );
    capture(
        child.stderr.take().unwrap(),
        Arc::clone(&stderr),
        options.limit,
        sender,
        false,
        options.tail,
        stderr_spool,
    );
    #[cfg(windows)]
    let writer = Some(stdin_writer);
    Ok(Started {
        child,
        stdout,
        stderr,
        events: Events::new(events),
        writer,
        #[cfg(windows)]
        completion,
        #[cfg(windows)]
        control,
        #[cfg(windows)]
        target_pid,
        #[cfg(unix)]
        owner,
        started,
        idle: options.idle,
    })
}

impl Started {
    #[cfg(windows)]
    pub(crate) fn bridge_pid(&self) -> u32 {
        self.target_pid
    }

    /// Waits for completion, for `timeout` measured from launch, or for
    /// `cancellation`; the latter two terminate the supervised boundary.
    pub fn supervise(
        self,
        timeout: Option<Duration>,
        cancellation: &Cancellation,
        observer: &mut impl FnMut(&[u8]) -> Result<(), String>,
    ) -> Result<Output, RunError> {
        let Self {
            mut child,
            stdout,
            stderr,
            mut events,
            writer,
            #[cfg(windows)]
            mut completion,
            #[cfg(windows)]
            control,
            #[cfg(windows)]
                target_pid: _,
            #[cfg(unix)]
                owner: _owner,
            started,
            idle,
        } = self;
        let mut last_activity = started;
        let mut status = None;
        let mut timed_out = false;
        let mut cancelled = false;
        let mut observer_error = None;
        let mut cleanup_error = None;
        #[cfg(windows)]
        let mut terminated = false;
        // Stops the boundary; Ok(true) when the job's emptiness was confirmed.
        macro_rules! terminate_owned {
            ($child:expr) => {{
                #[cfg(windows)]
                {
                    windows::terminate($child, control.as_ref(), &mut completion)
                }
                #[cfg(unix)]
                {
                    unix::terminate($child).map(|()| false)
                }
            }};
        }

        loop {
            events.collect();
            for chunk in events.chunks() {
                last_activity = Instant::now();
                if observer_error.is_none() {
                    observer_error = observer(&chunk).err();
                }
            }
            if observer_error.is_some()
                || stdout.lock().unwrap().error.is_some()
                || stderr.lock().unwrap().error.is_some()
            {
                #[cfg(windows)]
                {
                    terminated = true;
                }
                cleanup_error = terminate_owned!(&mut child).err();
                break;
            }
            if status.is_none() {
                match child.try_wait() {
                    Ok(value) => status = value,
                    Err(error) => {
                        let cleanup = terminate_owned!(&mut child).err();
                        return Err(format!(
                            "Could not wait for process: {error}{}",
                            cleanup.map_or(String::new(), |error| format!("; {error}"))
                        )
                        .into());
                    }
                }
            }
            if status.is_some()
                && events.readers_finished()
                && writer.as_ref().is_none_or(JoinHandle::is_finished)
            {
                break;
            }
            if cancellation.requested()
                || timeout.is_some_and(|timeout| started.elapsed() >= timeout)
                || idle.is_some_and(|idle| last_activity.elapsed() >= idle)
            {
                cancelled = cancellation.requested();
                timed_out = !cancelled;
                #[cfg(windows)]
                {
                    terminated = true;
                }
                let confirmed = match terminate_owned!(&mut child) {
                    Ok(confirmed) => confirmed,
                    Err(error) => {
                        cleanup_error = Some(error);
                        false
                    }
                };
                #[cfg(unix)]
                {
                    status = child.try_wait().ok().flatten().or(status);
                }
                // An inherited pipe must not make an already timed-out command block forever.
                let drain_limit = if confirmed {
                    Duration::from_secs(5)
                } else {
                    Duration::from_millis(250)
                };
                let deadline = Instant::now() + drain_limit;
                while !events.readers_finished() {
                    let remaining = deadline.saturating_duration_since(Instant::now());
                    if remaining.is_zero() {
                        break;
                    }
                    events.wait(remaining);
                }
                if !events.readers_finished() {
                    cleanup_error.get_or_insert_with(|| {
                        "Process cleanup could not be confirmed: output capture remains active"
                            .into()
                    });
                }
                break;
            }
            // Output and stream ends wake the loop at once; the interval
            // bounds how late exit, cancellation and deadlines are noticed.
            events.wait(Duration::from_millis(10));
        }
        #[cfg(windows)]
        if terminated {
            windows::reap(&mut child);
        }

        events.collect();
        for chunk in events.chunks() {
            if observer_error.is_none() {
                observer_error = observer(&chunk).err();
            }
        }
        let cleanup = |error: Option<String>| error.map_or(String::new(), |e| format!("; {e}"));
        if let Some(error) = observer_error {
            return Err(format!("{error}{}", cleanup(cleanup_error)).into());
        }
        let mut stdout = stdout.lock().unwrap();
        let mut stderr = stderr.lock().unwrap();
        // A failed capture is why the run stopped; cleanup only adds detail.
        if let Some(error) = stdout.error.as_ref().or(stderr.error.as_ref()) {
            return Err(format!(
                "Could not read process output: {error}{}",
                cleanup(cleanup_error)
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

        // A stopped run consumed the report while terminating; the
        // supervisor's exit code is not the command's then.
        #[cfg(windows)]
        let native_exit_code = match completion {
            _ if terminated => None,
            Some(report) => Some(windows::completion(report).map_err(RunError::from)?),
            None => status.and_then(|status| status.code()),
        };

        let input_error = if !timed_out && !cancelled {
            writer
                .filter(JoinHandle::is_finished)
                .and_then(|writer| match writer.join() {
                    Ok(Ok(())) => None,
                    Ok(Err(error)) => Some(format!("Could not send process input: {error}")),
                    Err(_) => Some("Process input worker failed".into()),
                })
        } else {
            None
        };
        Ok(Output {
            stdout: std::mem::take(&mut stdout.bytes),
            stderr: std::mem::take(&mut stderr.bytes),
            input_error,
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
            stdout_bytes: stdout.total_bytes,
            stderr_bytes: stderr.total_bytes,
            stdout_truncated: stdout.truncated,
            cancelled,
        })
    }
}

#[cfg(all(test, windows))]
mod env_tests;
#[cfg(all(test, windows))]
mod leaf_tests;
#[cfg(all(test, windows))]
mod stop_tests;
#[cfg(test)]
mod tests;
