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
pub(crate) mod duplex;
mod root;
mod supervise;
#[cfg(all(test, unix))]
pub(crate) use capture::Probe as CaptureProbe;
use capture::{Capture, Events, capture};
#[cfg(test)]
pub use root::Remaining;
pub use root::{RootExit, Watch};
#[cfg(unix)]
pub(crate) use unix::sandbox_policy;
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
#[cfg(any(unix, test))]
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
#[cfg(test)]
fn run_logged_input(
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
    #[cfg(unix)]
    pty_cleanup_receipt: Option<std::fs::File>,
    started: Instant,
    idle: Option<Duration>,
    watch: Watch,
}

#[cfg(unix)]
impl Started {
    pub(crate) fn expect_pty_cleanup_receipt(&mut self, receipt: std::fs::File) {
        self.pty_cleanup_receipt = Some(receipt);
    }
}

#[cfg(all(test, unix))]
impl Started {
    pub(crate) fn capture_probe(&self) -> CaptureProbe {
        CaptureProbe([Arc::clone(&self.stdout), Arc::clone(&self.stderr)])
    }
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
        stdin: _,
    } = unix::spawn(command, input, cancellation)?;
    #[cfg(windows)]
    let windows::Spawned {
        mut child,
        stdout: child_stdout,
        writer: stdin_writer,
        completion,
        control,
        target_pid,
        stdin: _,
        watch,
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
    // Unix observes its root directly: the guardian executes the command.
    #[cfg(unix)]
    let watch = Watch::default();
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
        #[cfg(unix)]
        pty_cleanup_receipt: None,
        started,
        idle: options.idle,
        watch,
    })
}

#[cfg(all(test, windows))]
mod env_tests;
#[cfg(all(test, windows))]
mod leaf_tests;
#[cfg(all(test, windows))]
mod stop_tests;
#[cfg(test)]
mod tests;
