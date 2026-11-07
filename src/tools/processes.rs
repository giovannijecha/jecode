//! Command processes owned by one conversation runtime.
//!
//! Every Bash and exec launch is one of these. A monitor thread owns the
//! supervised boundary until completion, the execution timeout or a stop; the
//! launching call only waits for it. Results read the saved, credential-masked
//! streams from per-process cursors, so every byte is either returned or
//! identified for `read`. An [`Observer`] reads the same streams without
//! touching those cursors.

#[cfg(test)]
use super::OUTPUT_LIMIT;
use super::required_string;
use crate::cancel::Cancellation;
use crate::json::Value;
use crate::process::Started;
use std::path::PathBuf;
use std::sync::atomic::AtomicUsize;
#[cfg(test)]
use std::sync::atomic::Ordering;
use std::sync::{Arc, Condvar, Mutex, Weak};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

mod entry;
mod lifecycle;
mod observer;
mod paging;
pub use lifecycle::Completion;
#[cfg(test)]
use observer::read_window;
pub use observer::{Observer, Page, Process, Window};
#[cfg(test)]
use paging::read_after;
use paging::{OutputMode, complete_prefix};

const STOP_WAIT: Duration = Duration::from_secs(10);
/// How long a launch without `wait_ms` holds the call before yielding.
pub(super) const DEFAULT_WAIT: Duration = Duration::from_secs(10);
pub(super) const DEFAULT_OUTPUT_LIMIT: usize = 16 * 1024;
pub(super) const MAX_OUTPUT_LIMIT: usize = 256 * 1024;

#[derive(Default)]
pub(super) struct Registry {
    // Replaced when the conversation ends, so observers never see a later one.
    scope: Mutex<Arc<Scope>>,
    #[cfg(test)]
    default_wait: Mutex<Option<Duration>>,
}

#[derive(Default)]
struct Scope {
    entries: Mutex<Vec<Arc<Entry>>>,
    /// Numbers terminal transitions no call reported.
    sequence: AtomicUsize,
    /// Wakes a local interface when a process starts or ends.
    listener: Mutex<Option<lifecycle::Listener>>,
}

pub(super) struct Stream {
    pub reference: String,
    pub path: PathBuf,
}

struct Entry {
    id: String,
    command: String,
    streams: [Stream; 2],
    stop: Cancellation,
    state: Mutex<State>,
    changed: Condvar,
    monitor: Mutex<Option<JoinHandle<()>>>,
    terminal: Option<Arc<crate::pty::Control>>,
    started: Instant,
    scope: Weak<Scope>,
}

#[derive(Default)]
struct State {
    outcome: Option<Outcome>,
    cursors: [u64; 2],
    finished: Option<Instant>,
    /// Calls that will report this process's state when they stop waiting.
    /// While there are any, a completion needs no announcement.
    waiters: usize,
    /// Advances when the user moves the current waits into the background.
    released: usize,
    /// The launching call has returned without a final result.
    returned: bool,
    /// Set when no call reported the terminal transition; orders completions.
    sequence: Option<usize>,
    /// The model has seen the terminal transition.
    reported: bool,
}

/// How one process ended, once its final output is available.
#[derive(Clone)]
pub(super) struct Outcome {
    pub status: &'static str,
    pub exit_code: Option<i32>,
    pub error: Option<String>,
    pub stdout_bytes: usize,
    pub stderr_bytes: usize,
    /// False when supervision itself failed and no output counts exist.
    pub supervised: bool,
}

/// Controls the initial wait and the lifetime of one launched process.
pub(super) struct Adoption<'a> {
    pub timeout: Option<Duration>,
    pub wait: Duration,
    /// The caller left the wait to Jecode: the wait keeps the turn's
    /// cancellation ownership and a finished command returns its final result.
    pub foreground: bool,
    pub output_limit: usize,
    pub cancellation: &'a Cancellation,
}

/// What a launching call returns.
pub(super) enum Launched {
    /// A foreground command ended within its wait.
    Finished(Outcome),
    Report(Result<Value, String>),
}

/// Why a call stopped waiting for a process.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum WaitEnd {
    /// The process finished or the wait elapsed.
    Elapsed,
    /// `cancellation` interrupted the wait.
    Cancelled,
    /// The user moved this wait into the background.
    Released,
}

impl Registry {
    fn scope(&self) -> Arc<Scope> {
        Arc::clone(&self.scope.lock().unwrap())
    }

    /// A local handle that never extends the conversation's processes.
    pub(super) fn observer(&self, redactor: crate::redact::Redactor) -> Observer {
        Observer::new(Arc::downgrade(&self.scope.lock().unwrap()), redactor)
    }

    pub(super) fn default_wait(&self) -> Duration {
        #[cfg(test)]
        if let Some(wait) = *self.default_wait.lock().unwrap() {
            return wait;
        }
        DEFAULT_WAIT
    }

    #[cfg(test)]
    pub(super) fn set_default_wait(&self, wait: Duration) {
        *self.default_wait.lock().unwrap() = Some(wait);
    }

    /// Adopts a launched process, then waits up to `wait` for completion.
    pub(super) fn adopt(
        &self,
        command: &str,
        streams: [Stream; 2],
        started: Started,
        adoption: Adoption<'_>,
    ) -> Launched {
        self.adopt_control(command, streams, started, adoption, None)
    }

    pub(super) fn adopt_terminal(
        &self,
        command: &str,
        streams: [Stream; 2],
        started: Started,
        adoption: Adoption<'_>,
        terminal: Arc<crate::pty::Control>,
    ) -> Result<Value, String> {
        match self.adopt_control(command, streams, started, adoption, Some(terminal)) {
            Launched::Report(report) => report,
            Launched::Finished(_) => unreachable!("only a foreground wait returns a final result"),
        }
    }

    fn adopt_control(
        &self,
        command: &str,
        streams: [Stream; 2],
        started: Started,
        adoption: Adoption<'_>,
        terminal: Option<Arc<crate::pty::Control>>,
    ) -> Launched {
        let Adoption {
            timeout,
            wait,
            foreground,
            output_limit,
            cancellation,
        } = adoption;
        let scope = self.scope();
        let entry = Arc::new(Entry {
            id: format!("process-{}", crate::sessions::identifier()),
            command: command.into(),
            streams,
            stop: Cancellation::default(),
            // The launching call reports whatever happens from here on.
            state: Mutex::new(State {
                waiters: 1,
                ..State::default()
            }),
            changed: Condvar::new(),
            monitor: Mutex::new(None),
            terminal,
            started: Instant::now(),
            scope: Arc::downgrade(&scope),
        });
        // Register before waiting so an interrupted wait leaves a discoverable process.
        scope.adopt(Arc::clone(&entry));
        let owner = Arc::clone(&entry);
        let monitor = thread::spawn(move || {
            let result = started.supervise(timeout, &owner.stop, &mut |_| Ok(()));
            if let Some(terminal) = &owner.terminal {
                terminal.close();
            }
            let mut state = owner.state.lock().unwrap();
            state.outcome = Some(Outcome::from(result));
            state.finished = Some(Instant::now());
            owner.announce(&mut state);
            drop(state);
            owner.changed.notify_all();
            owner.notify();
        });
        *entry.monitor.lock().unwrap() = Some(monitor);
        // The launching waiter was registered with the entry at generation 0.
        let end = entry.wait_held(wait, cancellation, Some(0));
        if !foreground {
            return Launched::Report(entry.report(end, output_limit, OutputMode::Next));
        }
        if end == WaitEnd::Cancelled {
            // Interrupting a foreground wait ends the command it was waiting for.
            entry.stop.cancel();
            entry.wait_held(STOP_WAIT, &Cancellation::default(), None);
        }
        if let Some(outcome) = entry.final_outcome() {
            return Launched::Finished(outcome);
        }
        let mut report = entry.report(end, output_limit, OutputMode::Next);
        // A process ending just now is a result, in the form of a process report.
        if let Ok(Value::Object(fields)) = &mut report
            && matches!(
                fields.get("status").and_then(Value::as_str),
                Some("running" | "stopping")
            )
        {
            fields.insert("yielded".into(), Value::Bool(true));
        }
        Launched::Report(report)
    }

    pub(super) fn execute(
        &self,
        arguments: &Value,
        cancellation: &Cancellation,
    ) -> Result<Value, String> {
        let action = required_string(arguments, "action")?;
        if action == "list" {
            return Ok(self.list());
        }
        if !matches!(action, "poll" | "stop" | "write" | "resize" | "interrupt") {
            return Err("action must be list, poll, stop, write, resize or interrupt".into());
        }
        let id = required_string(arguments, "process_id")?;
        if matches!(action, "write" | "resize" | "interrupt") {
            let _ = super::shell::protect_paths(arguments)?;
        }
        let output_limit = output_limit(arguments)?;
        let output_mode = OutputMode::parse(arguments)?;
        let entry = self
            .scope()
            .entries
            .lock()
            .unwrap()
            .iter()
            .find(|entry| entry.id == id)
            .cloned()
            .ok_or_else(|| {
                format!(
                    "No process {id} belongs to this conversation runtime. Processes end when Jecode exits or the conversation changes and are never restarted; their saved output references remain readable. Use action list for current processes."
                )
            })?;
        if matches!(action, "write" | "resize" | "interrupt") {
            if entry.state.lock().unwrap().outcome.is_some() || entry.stop.requested() {
                return Err("The process has ended or is stopping".into());
            }
            let terminal = entry
                .terminal
                .as_ref()
                .ok_or("This process has no interactive terminal")?;
            match action {
                "write" => {
                    let bytes = match (arguments.get("text"), arguments.get("key")) {
                        (Some(Value::String(text)), None) => text.as_bytes(),
                        (None, Some(Value::String(key))) => crate::pty::key(key)?,
                        _ => return Err("write needs either text or key".into()),
                    };
                    if arguments.get("key").and_then(Value::as_str) == Some("Ctrl+C") {
                        terminal.interrupt(cancellation)?;
                    } else {
                        terminal.write(bytes, cancellation)?;
                    }
                }
                "resize" => {
                    if arguments.get("cols").is_none() || arguments.get("rows").is_none() {
                        return Err("resize needs cols and rows".into());
                    }
                    terminal.resize(crate::pty::size(arguments)?, cancellation)?;
                }
                _ => terminal.interrupt(cancellation)?,
            }
        }
        let end = if action == "stop" {
            entry.stop.cancel();
            entry.wait(STOP_WAIT, cancellation)
        } else {
            let wait = wait_argument(arguments)?.unwrap_or(Duration::ZERO);
            entry.wait(wait, cancellation)
        };
        entry.report(end, output_limit, output_mode)
    }

    fn list(&self) -> Value {
        let scope = self.scope();
        let entries = scope.entries.lock().unwrap();
        Value::object([(
            "processes",
            Value::Array(
                entries
                    .iter()
                    .map(|entry| {
                        let mut state = entry.state.lock().unwrap();
                        // A final status here is the model learning the outcome.
                        state.reported |= state.outcome.is_some();
                        let mut fields = entry.status(&state);
                        fields.insert("command".into(), Value::string(&entry.command));
                        fields.insert(
                            "elapsed_seconds".into(),
                            Value::number(entry.elapsed(&state).as_secs()),
                        );
                        for (stream, name) in entry.streams.iter().zip(["stdout", "stderr"]) {
                            fields.insert(format!("{name}_file"), Value::string(&stream.reference));
                        }
                        Value::Object(fields)
                    })
                    .collect(),
            ),
        )])
    }

    /// Stops every owned process and forgets them; used when the conversation ends.
    pub(super) fn stop_all(&self) {
        let scope = std::mem::take(&mut *self.scope.lock().unwrap());
        let entries = std::mem::take(&mut *scope.entries.lock().unwrap());
        for entry in &entries {
            entry.stop.cancel();
        }
        for entry in &entries {
            if let Some(monitor) = entry.monitor.lock().unwrap().take() {
                let _ = monitor.join();
            }
        }
    }
}

impl Drop for Registry {
    fn drop(&mut self) {
        self.stop_all();
    }
}

pub(super) fn wait_argument(arguments: &Value) -> Result<Option<Duration>, String> {
    arguments
        .get("wait_ms")
        .map(|value| {
            value
                .as_usize()
                .map(|milliseconds| Duration::from_millis(milliseconds as u64))
                .ok_or_else(|| "wait_ms must be a nonnegative integer".to_string())
        })
        .transpose()
}

pub(super) fn output_limit(arguments: &Value) -> Result<usize, String> {
    arguments
        .get("output_limit_bytes")
        .map(|value| {
            value
                .as_usize()
                .filter(|value| *value > 0)
                .ok_or_else(|| "output_limit_bytes must be a positive integer".to_string())
        })
        .transpose()
        .map(|value| value.unwrap_or(DEFAULT_OUTPUT_LIMIT).min(MAX_OUTPUT_LIMIT))
}

#[cfg(test)]
mod budget_tests;
#[cfg(test)]
mod tests;
