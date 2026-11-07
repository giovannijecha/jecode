//! Local inspection and stop control for one conversation's processes.
//!
//! Reads take caller-supplied byte windows over the saved, credential-masked
//! streams, so they never advance the cursors behind the model's results.
//! The handle is weak: it neither extends processes nor survives the
//! conversation that owned them.

use super::{Completion, Entry, Scope, complete_prefix};
use crate::redact::Redactor;
use crate::tools::OUTPUT_LIMIT;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use std::sync::{Arc, Weak};
use std::time::{Duration, Instant};

const ENDED: &str = "The conversation that owned these processes has ended";
const STREAMS: [&str; 2] = ["stdout", "stderr"];

#[derive(Clone)]
pub struct Observer {
    scope: Weak<Scope>,
    redactor: Redactor,
}

/// One process as the registry reports it now.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Process {
    pub id: String,
    pub command: String,
    pub status: &'static str,
    pub exit_code: Option<i32>,
    pub error: Option<String>,
    pub started: Instant,
    pub finished: Option<Instant>,
    /// A tool call is waiting for it, and can be released into the background.
    pub awaited: bool,
}

impl Process {
    pub fn active(&self) -> bool {
        matches!(self.status, "running" | "stopping")
    }

    /// How long it ran, or has been running.
    pub fn elapsed(&self) -> Duration {
        self.finished
            .map_or_else(|| self.started.elapsed(), |end| end - self.started)
    }

    /// The status, with the exit code when one is known.
    pub fn label(&self) -> String {
        match (self.status, self.exit_code) {
            ("exited", Some(code)) => format!("exited {code}"),
            ("timed_out", _) => "timed out".into(),
            ("unknown", _) => "outcome unknown".into(),
            (status, _) => status.into(),
        }
    }
}

/// A bounded byte range of a saved stream.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Window {
    /// The newest bytes.
    Tail,
    /// Bytes that end at this offset.
    Before(u64),
    /// Bytes that start at this offset.
    After(u64),
}

/// Visible text from saved bytes in `start..end` of a stream of `length` bytes.
/// Masking may change the text length without changing saved-byte offsets.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Page {
    pub text: String,
    pub start: u64,
    pub end: u64,
    pub length: u64,
}

impl Observer {
    fn terminal(&self, id: &str) -> Result<std::sync::Arc<crate::pty::Control>, String> {
        let scope = self.scope.upgrade().ok_or(ENDED)?;
        let entries = scope.entries.lock().unwrap();
        let entry = entries
            .iter()
            .find(|entry| entry.id == id)
            .ok_or_else(|| format!("No process {id} belongs to this conversation runtime"))?;
        if entry.stop.requested() || entry.state.lock().unwrap().outcome.is_some() {
            return Err("The process has ended or is stopping".into());
        }
        entry
            .terminal
            .clone()
            .ok_or_else(|| "This process has no interactive terminal".into())
    }

    pub fn terminal_size(&self, id: &str) -> Option<(u16, u16)> {
        let scope = self.scope.upgrade()?;
        let entries = scope.entries.lock().unwrap();
        entries
            .iter()
            .find(|entry| entry.id == id)?
            .terminal
            .as_ref()
            .map(|terminal| terminal.size())
    }

    /// Writes local user input without creating a model message or tool call.
    pub fn write_direct(&self, id: &str, bytes: &[u8]) -> Result<(), String> {
        self.terminal(id)?.write_direct(bytes)
    }

    pub fn interrupt_direct(&self, id: &str) -> Result<(), String> {
        self.terminal(id)?.interrupt_direct()
    }

    pub fn resize_direct(&self, id: &str, size: (u16, u16)) -> Result<(), String> {
        self.terminal(id)?.resize_direct(size)
    }

    pub(super) fn new(scope: Weak<Scope>, redactor: Redactor) -> Self {
        Self { scope, redactor }
    }

    /// Whether both handles observe the same conversation runtime.
    pub fn same(&self, other: &Self) -> bool {
        Weak::ptr_eq(&self.scope, &other.scope)
    }

    /// Active processes in launch order, then finished ones, the latest
    /// first; `None` once the owning conversation has ended.
    pub fn processes(&self) -> Option<Vec<Process>> {
        let scope = self.scope.upgrade()?;
        let entries = scope.entries.lock().unwrap();
        let mut processes: Vec<_> = entries.iter().map(|entry| self.process(entry)).collect();
        processes.sort_by_key(|process| (!process.active(), std::cmp::Reverse(process.finished)));
        Some(processes)
    }

    /// Processes still running after their launching call returned.
    pub fn background_count(&self) -> usize {
        let Some(scope) = self.scope.upgrade() else {
            return 0;
        };
        let entries = scope.entries.lock().unwrap();
        entries
            .iter()
            .filter(|entry| {
                let state = entry.state.lock().unwrap();
                state.outcome.is_none() && state.returned
            })
            .count()
    }

    /// Terminal transitions no call reported, after this reader's own
    /// `cursor`; independent of what the model has received.
    pub fn completions_since(&self, cursor: usize) -> (usize, Vec<Completion>) {
        let Some(scope) = self.scope.upgrade() else {
            return (cursor, Vec::new());
        };
        let completions = scope.completions_after(cursor, &self.redactor, |_| true);
        let next = completions.last().map_or(cursor, |last| last.sequence);
        (next, completions)
    }

    /// Calls `listener` when a process of this conversation starts or ends.
    pub fn on_change(&self, listener: Arc<dyn Fn() + Send + Sync>) {
        if let Some(scope) = self.scope.upgrade() {
            scope.set_listener(listener);
        }
    }

    /// Returns control from any call waiting for this process, leaving the
    /// process running; nothing restarts and no output is consumed.
    pub fn move_to_background(&self, id: &str) -> Result<Process, String> {
        let scope = self.scope.upgrade().ok_or(ENDED)?;
        let entry = scope
            .entries
            .lock()
            .unwrap()
            .iter()
            .find(|entry| entry.id == id)
            .cloned()
            .ok_or_else(|| {
                self.redactor.text(&format!(
                    "No process {id} belongs to this conversation runtime"
                ))
            })?;
        {
            let mut state = entry.state.lock().unwrap();
            if state.outcome.is_some() || state.waiters == 0 {
                return Err("No call is waiting for this process".into());
            }
            state.released = state.released.wrapping_add(1);
        }
        entry.changed.notify_all();
        Ok(self.process(&entry))
    }

    /// Requests a stop without waiting for supervision or reading model output.
    /// A finished process returns its final state unchanged.
    pub fn request_stop(&self, id: &str) -> Result<Process, String> {
        let scope = self.scope.upgrade().ok_or(ENDED)?;
        let entry = scope
            .entries
            .lock()
            .unwrap()
            .iter()
            .find(|entry| entry.id == id)
            .cloned()
            .ok_or_else(|| {
                self.redactor.text(&format!(
                    "No process {id} belongs to this conversation runtime"
                ))
            })?;
        entry.request_stop();
        Ok(self.process(&entry))
    }

    /// Reads `window` of stream 0 (stdout) or 1 (stderr).
    pub fn read(&self, id: &str, stream: usize, window: Window) -> Result<Page, String> {
        let name = STREAMS.get(stream).ok_or("Unknown process stream")?;
        let scope = self.scope.upgrade().ok_or(ENDED)?;
        // Holding the list lets cleanup wait for this bounded read; once
        // cleanup has taken the list, no further read can begin.
        let entries = scope.entries.lock().unwrap();
        let entry = entries
            .iter()
            .find(|entry| entry.id == id)
            .ok_or_else(|| format!("No process {id} belongs to this conversation runtime"))?;
        // A final state read first guarantees that the saved stream is complete.
        let finished = entry.state.lock().unwrap().outcome.is_some();
        read_window(
            &entry.streams[stream].path,
            window,
            finished,
            &self.redactor,
        )
        .map_err(|error| {
            self.redactor
                .text(&format!("Could not read saved {name} for {id}: {error}"))
        })
    }

    fn process(&self, entry: &Entry) -> Process {
        let state = entry.state.lock().unwrap();
        let outcome = state.outcome.as_ref();
        Process {
            id: entry.id.clone(),
            command: self.redactor.text(&entry.command),
            status: entry.status_name(&state),
            exit_code: outcome.and_then(|outcome| outcome.exit_code),
            error: outcome
                .and_then(|outcome| outcome.error.as_deref())
                .map(|error| self.redactor.text(error)),
            started: entry.started,
            finished: state.finished,
            awaited: state.outcome.is_none() && state.waiters > 0,
        }
    }
}

/// Reads at most `OUTPUT_LIMIT` saved bytes; see `read_bounded`.
pub(super) fn read_window(
    path: &Path,
    window: Window,
    finished: bool,
    redactor: &Redactor,
) -> std::io::Result<Page> {
    read_bounded(path, window, finished, redactor, OUTPUT_LIMIT)
}

/// Reads at most `limit` saved bytes. The redactor may read bounded
/// context around the window; its replacement text does not change offsets.
/// A window cut by the limit starts and ends on UTF-8 boundaries; while the
/// process runs, a trailing incomplete sequence waits for a later read.
pub(super) fn read_bounded(
    path: &Path,
    window: Window,
    finished: bool,
    redactor: &Redactor,
    limit: usize,
) -> std::io::Result<Page> {
    let mut file = File::open(path)?;
    let length = file.metadata()?.len();
    let limit = limit as u64;
    let (mut start, end) = match window {
        Window::Tail => (length.saturating_sub(limit), length),
        Window::Before(end) => {
            let end = end.min(length);
            (end.saturating_sub(limit), end)
        }
        Window::After(start) => {
            let start = start.min(length);
            (start, start.saturating_add(limit).min(length))
        }
    };
    file.seek(SeekFrom::Start(start))?;
    let mut bytes = Vec::new();
    (&mut file).take(end - start).read_to_end(&mut bytes)?;
    if start > 0 && !matches!(window, Window::After(_)) {
        let skip = bytes
            .iter()
            .take(3)
            .take_while(|byte| **byte & 0xC0 == 0x80)
            .count();
        bytes.drain(..skip);
        start += skip as u64;
    }
    if !finished || end < length {
        bytes.truncate(complete_prefix(&bytes));
    }
    let mut end = start + bytes.len() as u64;
    let text = if let Some(overlap) = redactor.boundary_overlap() {
        let overlap = overlap as u64;
        let context_start = start.saturating_sub(overlap);
        let context_end = end.saturating_add(overlap).min(length);
        file.seek(SeekFrom::Start(context_start))?;
        let mut context = Vec::new();
        (&mut file)
            .take(context_end - context_start)
            .read_to_end(&mut context)?;
        if !finished {
            let (safe, _) = redactor.byte_prefix(&context, false);
            end = end.min((context_start + safe as u64).max(start));
        }
        redactor.byte_window(
            &context,
            (start - context_start) as usize,
            (end - context_start) as usize,
        )
    } else {
        bytes
    };
    Ok(Page {
        text: String::from_utf8_lossy(&text).into_owned(),
        start,
        end,
        length,
    })
}
