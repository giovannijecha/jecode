//! Persistent command processes owned by one conversation runtime.
//!
//! Each process keeps the supervised boundary of a foreground command; a
//! monitor thread owns it until completion, the execution timeout or a stop.
//! Results read the saved, credential-masked streams from per-process
//! cursors, so every byte is either returned or identified for `read`.
//! An [`Observer`] reads the same streams without touching those cursors.

#[cfg(test)]
use super::OUTPUT_LIMIT;
use super::required_string;
use crate::cancel::Cancellation;
use crate::json::Value;
use crate::process::{RunError, Started};
use std::collections::BTreeMap;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

mod observer;
#[cfg(test)]
use observer::read_window;
pub use observer::{Observer, Page, Process, Window};

const STOP_WAIT: Duration = Duration::from_secs(10);
pub(super) const DEFAULT_OUTPUT_LIMIT: usize = 16 * 1024;

#[derive(Default)]
pub(super) struct Registry {
    // Replaced when the conversation ends, so observers never see a later one.
    scope: Mutex<Arc<Scope>>,
}

#[derive(Default)]
struct Scope {
    entries: Mutex<Vec<Arc<Entry>>>,
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
}

#[derive(Default)]
struct State {
    outcome: Option<Outcome>,
    cursors: [u64; 2],
}

struct Outcome {
    status: &'static str,
    exit_code: Option<i32>,
    error: Option<String>,
}

/// Controls the initial wait and the lifetime of one launched process.
pub(super) struct Adoption<'a> {
    pub timeout: Option<Duration>,
    pub wait: Duration,
    pub output_limit: usize,
    pub cancellation: &'a Cancellation,
}

impl Registry {
    fn scope(&self) -> Arc<Scope> {
        Arc::clone(&self.scope.lock().unwrap())
    }

    /// A local handle that never extends the conversation's processes.
    pub(super) fn observer(&self, redactor: crate::redact::Redactor) -> Observer {
        Observer::new(Arc::downgrade(&self.scope.lock().unwrap()), redactor)
    }

    /// Adopts a launched process, then waits up to `wait` for completion.
    pub(super) fn adopt(
        &self,
        command: &str,
        streams: [Stream; 2],
        started: Started,
        adoption: Adoption<'_>,
    ) -> Result<Value, String> {
        let Adoption {
            timeout,
            wait,
            output_limit,
            cancellation,
        } = adoption;
        let entry = Arc::new(Entry {
            id: format!("process-{}", crate::sessions::identifier()),
            command: command.into(),
            streams,
            stop: Cancellation::default(),
            state: Mutex::default(),
            changed: Condvar::new(),
            monitor: Mutex::new(None),
        });
        // Register before waiting so an interrupted wait leaves a discoverable process.
        let scope = self.scope();
        scope.entries.lock().unwrap().push(Arc::clone(&entry));
        let owner = Arc::clone(&entry);
        let monitor = thread::spawn(move || {
            let result = started.supervise(timeout, &owner.stop, &mut |_| Ok(()));
            let mut state = owner.state.lock().unwrap();
            state.outcome = Some(Outcome::from(result));
            drop(state);
            owner.changed.notify_all();
        });
        *entry.monitor.lock().unwrap() = Some(monitor);
        let interrupted = entry.wait(wait, cancellation);
        entry.report(interrupted, output_limit)
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
        if !matches!(action, "poll" | "stop") {
            return Err("action must be list, poll or stop".into());
        }
        let id = required_string(arguments, "process_id")?;
        let output_limit = output_limit(arguments)?;
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
        let interrupted = if action == "stop" {
            entry.stop.cancel();
            entry.wait(STOP_WAIT, cancellation)
        } else {
            let wait = wait_argument(arguments)?.unwrap_or(Duration::ZERO);
            entry.wait(wait, cancellation)
        };
        entry.report(interrupted, output_limit)
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
                        let state = entry.state.lock().unwrap();
                        let mut fields = entry.status(&state);
                        fields.insert("command".into(), Value::string(&entry.command));
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

impl Entry {
    /// Returns true when `cancellation` interrupted the wait.
    fn wait(&self, wait: Duration, cancellation: &Cancellation) -> bool {
        // An unrepresentable deadline waits until completion or cancellation.
        let deadline = Instant::now().checked_add(wait);
        let mut state = self.state.lock().unwrap();
        while state.outcome.is_none() {
            if cancellation.requested() {
                return true;
            }
            let slice = deadline.map_or(Duration::from_millis(20), |deadline| {
                deadline
                    .saturating_duration_since(Instant::now())
                    .min(Duration::from_millis(20))
            });
            if slice.is_zero() {
                break;
            }
            state = self.changed.wait_timeout(state, slice).unwrap().0;
        }
        false
    }

    fn status_name(&self, state: &State) -> &'static str {
        match &state.outcome {
            Some(outcome) => outcome.status,
            None if self.stop.requested() => "stopping",
            None => "running",
        }
    }

    fn status(&self, state: &State) -> BTreeMap<String, Value> {
        let mut fields = BTreeMap::new();
        fields.insert("process_id".into(), Value::string(&self.id));
        fields.insert("status".into(), Value::string(self.status_name(state)));
        if let Some(outcome) = &state.outcome {
            if outcome.exit_code.is_some() || outcome.status == "exited" {
                fields.insert(
                    "exit_code".into(),
                    outcome.exit_code.map_or(Value::Null, Value::number),
                );
            }
            if let Some(error) = &outcome.error {
                fields.insert("error".into(), Value::string(error));
                fields.insert("outcome".into(), Value::string("unknown"));
            }
        }
        fields
    }

    /// Reports state, then output after each stream cursor. State is read
    /// first: a final state guarantees that the saved streams are complete.
    fn report(&self, interrupted: bool, output_limit: usize) -> Result<Value, String> {
        let mut state = self.state.lock().unwrap();
        let mut fields = self.status(&state);
        let finished = state.outcome.is_some();
        let mut cursors = state.cursors;
        for (index, name) in ["stdout", "stderr"].into_iter().enumerate() {
            let stream = &self.streams[index];
            let from = state.cursors[index];
            let chunk = read_after(&stream.path, from, finished, output_limit)
                .map_err(|error| format!("Could not read saved {name} for {}: {error}", self.id))?;
            cursors[index] = chunk.next;
            fields.insert(name.into(), Value::string(chunk.text));
            fields.insert(format!("{name}_offset"), Value::number(chunk.offset));
            fields.insert(format!("{name}_next_offset"), Value::number(chunk.next));
            fields.insert(
                format!("{name}_remaining_bytes"),
                Value::number(chunk.length - chunk.next),
            );
            fields.insert(
                format!("{name}_skipped_bytes"),
                Value::number(chunk.offset - from),
            );
            fields.insert(format!("{name}_file"), Value::string(&stream.reference));
        }
        state.cursors = cursors;
        fields.insert("output_limit_bytes".into(), Value::number(output_limit));
        if interrupted {
            fields.insert("wait_cancelled".into(), Value::Bool(true));
        }
        Ok(Value::Object(fields))
    }
}

impl From<Result<crate::process::Output, RunError>> for Outcome {
    fn from(result: Result<crate::process::Output, RunError>) -> Self {
        let (status, exit_code, error) = match result {
            Ok(output) if output.cancelled => ("stopped", None, None),
            Ok(output) if output.timed_out => ("timed_out", None, None),
            Ok(output) if output.input_error.is_some() => (
                "unknown",
                output.exit_code,
                Some(format!(
                    "The command started but input delivery failed: {}. Execution may already have changed the workspace; inspect it before repeating the command.",
                    output.input_error.unwrap()
                )),
            ),
            Ok(output) => ("exited", output.exit_code, None),
            Err(error) => (
                "unknown",
                None,
                Some(format!(
                    "The command started but its completion or cleanup could not be confirmed: {}. Execution may already have changed the workspace; inspect it before repeating the command.",
                    error.message
                )),
            ),
        };
        Self {
            status,
            exit_code,
            error,
        }
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
        .map(|value| value.unwrap_or(DEFAULT_OUTPUT_LIMIT))
}

struct Chunk {
    text: String,
    offset: u64,
    next: u64,
    length: u64,
}

/// Reads the next bounded window without advancing past unread bytes.
/// A trailing incomplete UTF-8 sequence waits while the process runs.
fn read_after(path: &Path, from: u64, finished: bool, limit: usize) -> std::io::Result<Chunk> {
    let mut file = File::open(path)?;
    let length = file.metadata()?.len().max(from);
    file.seek(SeekFrom::Start(from))?;
    let mut bytes = Vec::new();
    file.take((length - from).min(limit.saturating_add(3) as u64))
        .read_to_end(&mut bytes)?;
    let target = bytes.len().min(limit);
    let mut end = complete_prefix(&bytes[..target]);
    if end == 0 && bytes.len() > target {
        // A one-byte budget must still make progress on a complete UTF-8 character.
        let width = match bytes[0] {
            0xC0..=0xDF => 2,
            0xE0..=0xEF => 3,
            0xF0..=0xF7 => 4,
            _ => 1,
        };
        if bytes.len() >= width {
            end = width;
        }
    }
    if finished && bytes.len() <= limit {
        end = bytes.len();
    }
    Ok(Chunk {
        text: String::from_utf8_lossy(&bytes[..end]).into_owned(),
        offset: from,
        next: from + end as u64,
        length,
    })
}

fn complete_prefix(bytes: &[u8]) -> usize {
    for back in 1..=bytes.len().min(4) {
        let index = bytes.len() - back;
        let width = match bytes[index] {
            0x80..=0xBF => continue,
            0xC0..=0xDF => 2,
            0xE0..=0xEF => 3,
            0xF0..=0xF7 => 4,
            _ => 1,
        };
        return if back < width { index } else { bytes.len() };
    }
    bytes.len()
}

#[cfg(test)]
mod budget_tests;
#[cfg(test)]
mod tests;
