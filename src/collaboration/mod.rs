//! Session-scoped collaborator receipts and live workers.

mod changes;
mod log;
#[cfg(test)]
mod numbering_tests;
mod placement;
mod restore;
mod runtime;
#[cfg(test)]
mod snapshot_tests;
#[cfg(test)]
mod tests;
mod tracked;
pub mod workspace;

const MAX_UPDATES: usize = 64;
const UPDATE_CHARS: usize = 2000;

use crate::{cancel::Cancellation, export::Archive, inbox::Inbox, json::Value, sessions::Handle};
use std::{
    collections::VecDeque,
    sync::{Arc, Condvar, Mutex, mpsc},
    thread::JoinHandle,
};
use tracked::Tracked;
use workspace::Workspace;

/// Shared collaboration state. A handle made for one agent call carries that
/// call's work label, which every receipt it writes records.
#[derive(Clone, Default)]
pub struct Observer(Arc<Shared>, Option<String>);

/// A record as an observer last read it: its id and revision.
pub type Seen = (String, u64);

#[derive(Default)]
struct Shared {
    state: Mutex<State>,
    changed: Condvar,
    parent: Mutex<Option<(Archive, Option<Handle>)>>,
}

#[derive(Default)]
struct State {
    records: Vec<Tracked<Record>>,
    receipts: Vec<Value>,
    workers: Vec<Worker>,
    last_number: usize,
    /// Turns and calls of the main agent running now; receipts record
    /// whether one was.
    turns: usize,
    calls: usize,
}

impl State {
    /// The record to change; taking it advances its revision.
    fn record(&mut self, id: &str) -> Option<&mut Record> {
        self.records
            .iter_mut()
            .find(|record| record.id == id)
            .map(|record| &mut **record)
    }

    /// The lowest number no open collaborator shows.
    fn free_number(&self) -> usize {
        (1..)
            .find(|number| {
                !self
                    .records
                    .iter()
                    .any(|record| !record.workspace_closed && record.number == *number)
            })
            .unwrap()
    }
}

/// The sequence in a collaborator's ID (`a7`), which numbered it before
/// shown numbers were reused.
fn sequence(record: &Value) -> usize {
    let id = record
        .get("id")
        .and_then(Value::as_str)
        .and_then(|id| id.strip_prefix('a'))
        .and_then(|id| id.parse().ok());
    let number = record.get("number").and_then(Value::as_usize);
    id.max(number).unwrap_or(0)
}

struct Worker {
    id: String,
    inbox: Inbox,
    cancellation: Cancellation,
    wake: mpsc::Sender<()>,
    task: Option<JoinHandle<()>>,
}

#[derive(Clone)]
struct Assignment {
    version: usize,
    instructions: String,
    context: String,
}

pub(crate) struct Spec {
    pub title: String,
    pub instructions: String,
    pub context: String,
    pub mode: workspace::Mode,
}

struct Record {
    id: String,
    number: usize,
    title: String,
    child_session: Option<String>,
    /// The running child's session events, which hold its file changes.
    session_events: Option<crate::export::Messages>,
    state: &'static str,
    workspace: Option<Workspace>,
    workspace_info: Value,
    workspace_mode: String,
    directory: String,
    workspace_closed: bool,
    sent_version: usize,
    received_version: usize,
    instructions: String,
    context: String,
    pending: VecDeque<Assignment>,
    activity: String,
    updates: Vec<String>,
    updates_truncated: bool,
    tools: Vec<Value>,
    /// Assignments, messages, calls and returns in the order they happened.
    log: Vec<log::Entry>,
    /// Message ordinals handed out; the latest `updates` keep their text.
    messages: usize,
    files: Vec<String>,
    checks: Vec<String>,
    report: Option<(usize, String)>,
    reviewed: bool,
    error: Option<String>,
}

impl Record {
    fn value(&self) -> Value {
        Value::object([
            ("id", Value::string(&self.id)),
            ("number", Value::number(self.number)),
            ("title", Value::string(&self.title)),
            (
                "child_session",
                self.child_session
                    .as_ref()
                    .map_or(Value::Null, Value::string),
            ),
            ("state", Value::string(self.state)),
            ("workspace", Value::string(&self.workspace_mode)),
            ("workspace_info", self.workspace_info.clone()),
            ("directory", Value::string(&self.directory)),
            ("workspace_closed", Value::Bool(self.workspace_closed)),
            ("sent_version", Value::number(self.sent_version)),
            ("received_version", Value::number(self.received_version)),
            ("instructions", Value::string(&self.instructions)),
            ("context", Value::string(&self.context)),
            (
                "pending_instruction",
                self.pending
                    .back()
                    .map_or(Value::Null, |a| Value::string(&a.instructions)),
            ),
            (
                "pending_context",
                self.pending
                    .back()
                    .map_or(Value::Null, |a| Value::string(&a.context)),
            ),
            ("activity", Value::string(&self.activity)),
            (
                "updates",
                Value::Array(self.updates.iter().map(Value::string).collect()),
            ),
            ("updates_truncated", Value::Bool(self.updates_truncated)),
            ("tools", Value::Array(self.tools.clone())),
            (
                "log",
                log::value(&self.log, self.messages, self.updates.len()),
            ),
            (
                "files",
                Value::Array(self.files.iter().map(Value::string).collect()),
            ),
            (
                "checks",
                Value::Array(self.checks.iter().map(Value::string).collect()),
            ),
            (
                "report",
                self.report.as_ref().map_or(Value::Null, |(version, text)| {
                    Value::object([
                        ("version", Value::number(*version)),
                        ("text", Value::string(text)),
                    ])
                }),
            ),
            ("reviewed", Value::Bool(self.reviewed)),
            (
                "error",
                self.error.as_ref().map_or(Value::Null, Value::string),
            ),
        ])
    }

    fn from_value(value: &Value) -> Result<Self, String> {
        let text = |name| {
            value
                .get(name)
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned()
        };
        let number = value
            .get("number")
            .and_then(Value::as_usize)
            .ok_or("Invalid collaborator number")?;
        let mut state = match text("state").as_str() {
            "running" | "stopping" => "unknown",
            "returned" => "returned",
            "failed" => "failed",
            "stopped" => "stopped",
            _ => "unknown",
        };
        let report = value.get("report").and_then(|report| {
            Some((
                report.get("version")?.as_usize()?,
                report.get("text")?.as_str()?.to_owned(),
            ))
        });
        let tools = value
            .get("tools")
            .and_then(Value::as_array)
            .unwrap_or(&[])
            .to_vec();
        let persisted = value
            .get("updates")
            .and_then(Value::as_array)
            .map_or(0, |updates| {
                updates.iter().filter_map(Value::as_str).count()
            });
        let (log, messages) = log::restore(value, persisted, tools.len());
        let workspace_info = value.get("workspace_info").cloned().unwrap_or(Value::Null);
        let workspace_closed = value.get("workspace_closed") == Some(&Value::Bool(true));
        let (workspace, workspace_error) = if workspace_closed {
            (None, None)
        } else {
            match workspace::Workspace::from_value(&workspace_info) {
                Ok(workspace) => (Some(workspace), None),
                Err(error) => {
                    state = "unknown";
                    (None, Some(format!("Workspace unavailable: {error}")))
                }
            }
        };
        Ok(Self {
            id: text("id"),
            number,
            title: text("title"),
            child_session: value
                .get("child_session")
                .and_then(Value::as_str)
                .map(str::to_owned),
            session_events: None,
            state,
            workspace,
            workspace_info,
            workspace_mode: text("workspace"),
            directory: text("directory"),
            workspace_closed,
            sent_version: value
                .get("sent_version")
                .and_then(Value::as_usize)
                .unwrap_or(0),
            received_version: value
                .get("received_version")
                .and_then(Value::as_usize)
                .unwrap_or(0),
            instructions: text("instructions"),
            context: text("context"),
            pending: value
                .get("pending_instruction")
                .and_then(Value::as_str)
                .map(|instructions| {
                    VecDeque::from([Assignment {
                        version: value
                            .get("sent_version")
                            .and_then(Value::as_usize)
                            .unwrap_or(0),
                        instructions: instructions.to_owned(),
                        context: text("pending_context"),
                    }])
                })
                .unwrap_or_default(),
            activity: text("activity"),
            updates: value
                .get("updates")
                .and_then(Value::as_array)
                .unwrap_or(&[])
                .iter()
                .rev()
                .take(MAX_UPDATES)
                .rev()
                .filter_map(Value::as_str)
                .map(|text| text.chars().take(UPDATE_CHARS).collect())
                .collect(),
            updates_truncated: value.get("updates_truncated") == Some(&Value::Bool(true))
                || value
                    .get("updates")
                    .and_then(Value::as_array)
                    .is_some_and(|updates| updates.len() > MAX_UPDATES),
            tools,
            log,
            messages,
            files: value
                .get("files")
                .and_then(Value::as_array)
                .unwrap_or(&[])
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect(),
            checks: value
                .get("checks")
                .and_then(Value::as_array)
                .unwrap_or(&[])
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect(),
            report,
            reviewed: value.get("reviewed") == Some(&Value::Bool(true)),
            error: workspace_error.or_else(|| {
                value
                    .get("error")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            }),
        })
    }
}

impl Observer {
    pub(crate) fn abort_created(&self, ids: &[String], gate: &Arc<(Mutex<bool>, Condvar)>) {
        let workers = {
            let mut state = self.0.state.lock().unwrap();
            for record in &mut state.records {
                if ids.contains(&record.id) {
                    record.state = "unknown";
                }
            }
            let mut workers = Vec::new();
            let mut index = 0;
            while index < state.workers.len() {
                if ids.contains(&state.workers[index].id) {
                    let worker = state.workers.remove(index);
                    worker.cancellation.cancel();
                    worker.inbox.wake();
                    let _ = worker.wake.send(());
                    workers.push(worker);
                } else {
                    index += 1;
                }
            }
            workers
        };
        let (lock, ready) = &**gate;
        *lock.lock().unwrap() = true;
        ready.notify_all();
        for mut worker in workers {
            if let Some(task) = worker.task.take() {
                let _ = task.join();
            }
        }
        let workspaces = {
            let mut state = self.0.state.lock().unwrap();
            let mut workspaces = Vec::new();
            state.records.retain(|record| {
                if ids.contains(&record.id) {
                    if let Some(workspace) = &record.workspace {
                        workspaces.push(workspace.clone());
                    }
                    false
                } else {
                    true
                }
            });
            workspaces
        };
        for workspace in workspaces {
            let _ = workspace.close();
        }
    }

    pub fn snapshot(&self) -> Vec<Value> {
        self.snapshot_since(&[])
            .unwrap_or_default()
            .into_iter()
            .filter_map(|(_, value)| value)
            .collect()
    }

    /// The records in order, when any changed since `seen`: each with its
    /// revision, and its value only where `seen` lacks that revision.
    /// Records carry every tool result, so unchanged ones are not copied.
    pub fn snapshot_since(&self, seen: &[Seen]) -> Option<Vec<(Seen, Option<Value>)>> {
        let records = {
            let state = self.0.state.lock().unwrap();
            if state.records.len() == seen.len()
                && state
                    .records
                    .iter()
                    .zip(seen)
                    .all(|(record, (id, revision))| {
                        record.id == *id && record.revision() == *revision
                    })
            {
                return None;
            }
            state
                .records
                .iter()
                .map(|record| {
                    let key = (record.id.clone(), record.revision());
                    let value = (!seen.contains(&key)).then(|| record.value());
                    (key, value)
                })
                .collect::<Vec<_>>()
        };
        let redactor = self
            .0
            .parent
            .lock()
            .unwrap()
            .as_ref()
            .map(|(archive, _)| archive.redactor.clone());
        let Some(redactor) = redactor else {
            return Some(records);
        };
        Some(
            records
                .into_iter()
                .map(|(key, value)| (key, value.map(|value| redactor.value(&value))))
                .collect(),
        )
    }

    pub fn events_since(&self, cursor: usize) -> (usize, Vec<Value>) {
        let state = self.0.state.lock().unwrap();
        (
            state.receipts.len(),
            state.receipts.iter().skip(cursor).cloned().collect(),
        )
    }

    pub fn latest_return_sequence(&self) -> usize {
        self.0
            .state
            .lock()
            .unwrap()
            .receipts
            .iter()
            .rev()
            .find(|event| event.get("kind").and_then(Value::as_str) == Some("returned"))
            .and_then(|event| event.get("sequence"))
            .and_then(Value::as_usize)
            .unwrap_or(0)
    }

    pub fn stop_all(&self) {
        let workers = {
            let mut state = self.0.state.lock().unwrap();
            let ids = state
                .workers
                .iter()
                .map(|worker| worker.id.clone())
                .collect::<Vec<_>>();
            for record in &mut state.records {
                if ids.contains(&record.id) && record.state == "running" {
                    record.state = "stopping";
                }
            }
            for worker in &state.workers {
                worker.cancellation.cancel();
                worker.inbox.wake();
                let _ = worker.wake.send(());
            }
            std::mem::take(&mut state.workers)
        };
        for mut worker in workers {
            if let Some(task) = worker.task.take() {
                let _ = task.join();
            }
        }
        let stopped = {
            let mut state = self.0.state.lock().unwrap();
            state
                .records
                .iter_mut()
                .filter(|record| record.state == "stopping")
                .map(|record| {
                    record.state = "stopped";
                    record.log.push(log::Entry::Stopped);
                    (
                        record.id.clone(),
                        record.received_version,
                        record.title.clone(),
                    )
                })
                .collect::<Vec<_>>()
        };
        for (id, version, title) in stopped {
            let _ = self.receipt("stopped", Some(&id), version, &title, Vec::new());
        }
    }
}
