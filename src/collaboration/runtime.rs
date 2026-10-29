mod control;
use super::{
    Assignment, Observer, Record, Spec, State, Tracked, Worker, log::Entry, workspace::Workspace,
};
use crate::{agent::Agent, attachments::Prompt, events::Event, json::Value};
use std::{
    collections::VecDeque,
    path::Path,
    sync::{Arc, Condvar, Mutex, mpsc},
    thread,
    time::Duration,
};

impl Observer {
    pub(crate) fn create(
        &self,
        project: &Path,
        spec: Spec,
        make_child: impl FnOnce(&Path) -> Result<Agent, String>,
        start_gate: Arc<(Mutex<bool>, Condvar)>,
    ) -> Result<Value, String> {
        let Spec {
            title,
            instructions,
            context,
            mode,
        } = spec;
        let (id, store) = {
            let mut state = self.0.state.lock().unwrap();
            let store = self
                .0
                .parent
                .lock()
                .unwrap()
                .as_ref()
                .and_then(|(_, handle)| handle.as_ref())
                .map(|handle| {
                    handle
                        .store()
                        .bucket()
                        .join("collaboration")
                        .join(handle.id())
                })
                .ok_or("Collaborators require session storage")?;
            state.last_number += 1;
            (format!("a{}", state.last_number), store)
        };
        let workspace = Workspace::prepare(project, &store, &id, mode)?;
        let child = match make_child(workspace.root()) {
            Ok(child) => child,
            Err(error) => {
                let _ = workspace.close();
                return Err(error);
            }
        };
        let child_session = child.sessions().map(|handle| handle.id());
        let session_events = Some(child.archive().events);
        let inbox = child.inbox();
        let cancellation = child.cancellation();
        let (wake, receive) = mpsc::channel();
        let record = Record {
            id: id.clone(),
            number: 0,
            title: title.clone(),
            child_session,
            session_events,
            state: "running",
            workspace_info: workspace.value(),
            workspace_mode: match workspace.mode() {
                super::workspace::Mode::Shared => "shared",
                super::workspace::Mode::Isolated => "isolated",
            }
            .into(),
            directory: workspace.root().to_string_lossy().into_owned(),
            workspace_closed: false,
            workspace: Some(workspace),
            sent_version: 1,
            received_version: 0,
            instructions: String::new(),
            context: String::new(),
            pending: VecDeque::from([Assignment {
                version: 1,
                instructions,
                context,
            }]),
            activity: "Starting".into(),
            updates: vec![],
            updates_truncated: false,
            tools: vec![],
            log: vec![],
            messages: 0,
            files: vec![],
            checks: vec![],
            report: None,
            reviewed: false,
            error: None,
        };
        let number = {
            // IDs stay unique. The shown number is the lowest no open
            // collaborator holds, taken as it joins so concurrent creations
            // never share one: after a closed one, the next is AGENT 1 again.
            let mut state = self.0.state.lock().unwrap();
            let number = state.free_number();
            state
                .records
                .push(Tracked::new(Record { number, ..record }));
            number
        };
        // The child outlives this call; its receipts carry no call label.
        let observer = Self(Arc::clone(&self.0), None);
        let thread_id = id.clone();
        let task = thread::Builder::new()
            .name(format!("jecode-agent-{id}"))
            .spawn(move || {
                let (lock, ready) = &*start_gate;
                let mut started = lock.lock().unwrap();
                while !*started {
                    started = ready.wait(started).unwrap();
                }
                drop(started);
                observer.run_child(thread_id, child, receive)
            });
        let task = match task {
            Ok(task) => task,
            Err(error) => {
                let record = self
                    .0
                    .state
                    .lock()
                    .unwrap()
                    .records
                    .pop()
                    .unwrap()
                    .into_inner();
                if let Some(workspace) = record.workspace {
                    let _ = workspace.close();
                }
                return Err(format!("Could not start collaborator: {error}"));
            }
        };
        self.0.state.lock().unwrap().workers.push(Worker {
            id: id.clone(),
            inbox,
            cancellation,
            wake,
            task: Some(task),
        });
        Ok(Value::object([
            ("id", Value::string(id)),
            ("number", Value::number(number)),
        ]))
    }

    fn run_child(&self, id: String, mut child: Agent, receive: mpsc::Receiver<()>) {
        loop {
            if child.cancellation().requested() {
                break;
            }
            let assignment = {
                let mut state = self.0.state.lock().unwrap();
                let Some(record) = state.record(&id) else {
                    break;
                };
                if record.state == "stopping" {
                    None
                } else {
                    let assignment = record.pending.pop_back();
                    if assignment.is_some() {
                        record.pending.clear();
                        // The selected assignment becomes the initial prompt.
                        // Updates sent after this handoff remain in the inbox.
                        child.inbox().update(|entries| {
                            entries.drain();
                        });
                    }
                    assignment
                }
            };
            let Some(assignment) = assignment else {
                if self
                    .0
                    .state
                    .lock()
                    .unwrap()
                    .records
                    .iter()
                    .any(|record| record.id == id && record.state == "stopping")
                {
                    break;
                }
                if receive.recv().is_err() {
                    break;
                }
                continue;
            };
            // Superseded sent revisions remain receipts, but only the newest is received.
            if self.received(&id, &assignment).is_err() {
                break;
            }
            let prompt = format!(
                "Assignment v{}: {}\n\nRelevant context:\n{}\n\nReport the changes, checks, and unresolved issues.",
                assignment.version, assignment.instructions, assignment.context
            );
            let mut report = String::new();
            let result = child.run_turn(Prompt::plain(prompt), &mut |event| {
                match event {
                    Event::Message { text } => {
                        self.message(&id, &text);
                        if !report.is_empty() {
                            report.push_str("\n\n");
                        }
                        report.push_str(&text);
                    }
                    Event::Streaming { text } => self.progress(&id, &text),
                    Event::ToolStarted {
                        id: call_id,
                        name,
                        arguments,
                    } => self.tool_started(&id, &call_id, &name, &arguments),
                    Event::ToolFinished {
                        id: call_id,
                        name,
                        summary,
                        result,
                    } => self.tool_finished(&id, &call_id, &name, &summary, &result),
                    Event::ToolChanged {
                        id: call_id,
                        change,
                    } => self.tool_changed(&id, &call_id, &change),
                    Event::Accepted { prompts } => {
                        let mut accepted = Vec::new();
                        {
                            let mut state = self.0.state.lock().unwrap();
                            if let Some(record) = state.record(&id) {
                                for _ in prompts {
                                    if let Some(assignment) = record.pending.pop_front() {
                                        accepted.push(assignment);
                                    }
                                }
                            }
                        }
                        for assignment in accepted {
                            self.received(&id, &assignment)?;
                        }
                    }
                    _ => {}
                }
                Ok(())
            });
            if self
                .0
                .state
                .lock()
                .unwrap()
                .records
                .iter()
                .any(|record| record.id == id && record.state == "unknown")
            {
                break;
            }
            let (kind, version, title, details) = {
                let mut state = self.0.state.lock().unwrap();
                let Some(record) = state.record(&id) else {
                    break;
                };
                if record.state == "stopping" {
                    record.state = "stopped";
                    record.log.push(Entry::Stopped);
                    (
                        "stopped",
                        record.received_version,
                        record.title.clone(),
                        Vec::new(),
                    )
                } else {
                    let version = record.received_version;
                    record.log.push(Entry::Returned(version));
                    record.report = Some((version, report.clone()));
                    record.reviewed = false;
                    record.activity = if result.is_ok() {
                        "Returned".into()
                    } else {
                        "Failed".into()
                    };
                    record.error = result.as_ref().err().cloned();
                    record.state = if !record.pending.is_empty() {
                        "running"
                    } else if result.is_ok() {
                        "returned"
                    } else {
                        "failed"
                    };
                    (
                        "returned",
                        version,
                        record.title.clone(),
                        result.err().into_iter().collect(),
                    )
                }
            };
            if let Err(error) = self.receipt(kind, Some(&id), version, &title, details) {
                self.persistence_failed(&id, error);
                break;
            }
            if kind == "stopped" {
                break;
            }
        }
        let stopped = {
            let mut state = self.0.state.lock().unwrap();
            state
                .records
                .iter_mut()
                .find(|record| record.id == id)
                .and_then(|record| {
                    if record.state == "stopping" {
                        record.state = "stopped";
                        record.log.push(Entry::Stopped);
                        Some((record.received_version, record.title.clone()))
                    } else {
                        None
                    }
                })
        };
        if let Some((version, title)) = stopped
            && let Err(error) = self.receipt("stopped", Some(&id), version, &title, vec![])
        {
            self.persistence_failed(&id, error);
        }
        self.0.changed.notify_all();
    }

    fn received(&self, id: &str, assignment: &Assignment) -> Result<(), String> {
        let title = {
            let mut state = self.0.state.lock().unwrap();
            let Some(record) = state.record(id) else {
                return Err("Unknown collaborator".into());
            };
            record.received_version = assignment.version;
            record.instructions = assignment.instructions.clone();
            record.context = assignment.context.clone();
            record.state = "running";
            record.activity = "Working".into();
            record.log.push(Entry::Assignment {
                version: assignment.version,
                instructions: assignment.instructions.clone(),
            });
            record.title.clone()
        };
        self.receipt("received", Some(id), assignment.version, &title, vec![])
            .map(|_| ())
            .inspect_err(|error| self.persistence_failed(id, error.clone()))
    }

    fn persistence_failed(&self, id: &str, error: String) {
        let mut state = self.0.state.lock().unwrap();
        if let Some(record) = state.record(id) {
            record.state = "unknown";
            record.error = Some(format!("Could not save collaborator receipt: {error}"));
            record.activity = "Outcome could not be confirmed".into();
        }
        self.0.changed.notify_all();
    }

    fn progress(&self, id: &str, text: &str) {
        let mut state = self.0.state.lock().unwrap();
        if let Some(record) = state.record(id) {
            record.activity = text.chars().take(200).collect();
            self.0.changed.notify_all();
        }
    }

    pub(super) fn message(&self, id: &str, text: &str) {
        let mut state = self.0.state.lock().unwrap();
        if let Some(record) = state.record(id) {
            record.activity = text.chars().take(200).collect();
            if record.updates.len() == super::MAX_UPDATES {
                record.updates.remove(0);
                record.updates_truncated = true;
            }
            record
                .updates
                .push(text.chars().take(super::UPDATE_CHARS).collect());
            record.log.push(Entry::Message(Some(record.messages)));
            record.messages += 1;
            self.0.changed.notify_all();
        }
    }

    pub(super) fn tool_started(&self, id: &str, call_id: &str, name: &str, arguments: &Value) {
        let mut state = self.0.state.lock().unwrap();
        if let Some(record) = state.record(id) {
            record.activity = format!("{name}: running");
            record.log.push(Entry::Tool(record.tools.len()));
            record.tools.push(Value::object([
                ("id", Value::string(call_id)),
                ("name", Value::string(name)),
                ("state", Value::string("running")),
                ("text", Value::string("")),
                ("arguments", arguments.clone()),
            ]));
            self.0.changed.notify_all();
        }
    }

    pub(super) fn tool_finished(
        &self,
        id: &str,
        call_id: &str,
        name: &str,
        summary: &str,
        result: &Value,
    ) {
        let mut state = self.0.state.lock().unwrap();
        if let Some(record) = state.record(id) {
            record.activity = format!("{name}: {summary}");
            if let Some(Value::Object(tool)) = record
                .tools
                .iter_mut()
                .rev()
                .find(|tool| tool.get("id").and_then(Value::as_str) == Some(call_id))
            {
                tool.insert(
                    "state".into(),
                    Value::string(if result.get("error").is_some() {
                        "failed"
                    } else {
                        "done"
                    }),
                );
                tool.insert("text".into(), Value::string(summary));
                tool.insert("result".into(), result.clone());
            }
            if matches!(name, "bash" | "exec") {
                record.checks.push(format!("{name}: {summary}"));
            }
            self.0.changed.notify_all();
        }
    }

    /// Records the changed path and its line counts, for receipts that list
    /// changed files with `+` / `−` counts.
    pub(super) fn tool_changed(
        &self,
        id: &str,
        call_id: &str,
        change: &crate::file_change::Change,
    ) {
        let mut state = self.0.state.lock().unwrap();
        let Some(record) = state.record(id) else {
            return;
        };
        if let Some(Value::Object(tool)) = record
            .tools
            .iter_mut()
            .rev()
            .find(|tool| tool.get("id").and_then(Value::as_str) == Some(call_id))
        {
            tool.insert(
                "change".into(),
                Value::object([
                    ("added", Value::number(change.added)),
                    ("removed", Value::number(change.removed)),
                    ("created", Value::Bool(change.created)),
                ]),
            );
            self.0.changed.notify_all();
        }
        if let Some(tool) = record
            .tools
            .iter()
            .rev()
            .find(|tool| tool.get("id").and_then(Value::as_str) == Some(call_id))
            && let Some(path) = tool
                .get("arguments")
                .and_then(|arguments| arguments.get("path"))
                .and_then(Value::as_str)
            && !record.files.iter().any(|file| file == path)
        {
            record.files.push(path.to_owned());
            self.0.changed.notify_all();
        }
    }
}
