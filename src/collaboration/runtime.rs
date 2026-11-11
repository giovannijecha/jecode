mod control;
use super::{
    Assignment, Observer, Record, Spec, State, Tracked, Worker, log::Entry, workspace::Workspace,
};
use crate::{agent::Agent, attachments::Prompt, events::Event, json::Value};
use std::{
    collections::VecDeque,
    path::Path,
    sync::{Arc, Mutex},
    time::Duration,
};

impl Observer {
    pub(crate) fn create(
        &self,
        project: &Path,
        spec: Spec,
        make_child: impl FnOnce(&str, &Path) -> Result<Agent, String>,
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
                .map(|handle| handle.store().collaboration(&handle.id()))
                .ok_or("Collaborators require session storage")?;
            state.last_number += 1;
            (format!("a{}", state.last_number), store)
        };
        let workspace = Workspace::prepare(project, &store, &id, mode)?;
        let omitted = workspace.omitted();
        let child = match make_child(&id, workspace.root()) {
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
            activity: "Waiting for a free slot".into(),
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
            state.queue.push_back(id.clone());
            state.workers.push(Worker {
                id: id.clone(),
                child: Arc::new(Mutex::new(child)),
                inbox,
                cancellation,
                ready: false,
                busy: false,
            });
            number
        };
        let mut created =
            Value::object([("id", Value::string(id)), ("number", Value::number(number))]);
        if let (Value::Object(fields), Some(omitted)) = (&mut created, omitted) {
            fields.insert("omitted_from_copy".into(), omitted);
        }
        Ok(created)
    }

    pub(super) fn run_child(&self, id: &str, child: &mut Agent) {
        // Select the newest revision only when this turn actually starts.
        // Updates before this handoff replace the initial prompt; later ones
        // enter the active turn through the inbox.
        let assignment = {
            let mut state = self.0.state.lock().unwrap();
            if child.cancellation().requested()
                && let Some(record) = state.record(id)
                && record.state == "running"
            {
                record.state = "stopping";
            }
            let record = state.record(id);
            if let Some(record) = record.filter(|record| record.state == "running") {
                let assignment = record.pending.back().cloned();
                if assignment.is_some() {
                    child.inbox().with(|entries| {
                        entries.drain();
                    });
                }
                assignment
            } else {
                None
            }
        };
        let Some(assignment) = assignment else {
            self.finish_stopped(id);
            return;
        };
        if self.received(id, &assignment).is_err() {
            self.finish_stopped(id);
            return;
        }
        let prompt = format!(
            "Assignment v{}: {}\n\nRelevant context:\n{}\n\nReport the changes, checks, and unresolved issues.",
            assignment.version, assignment.instructions, assignment.context
        );
        let mut report = String::new();
        let result = child.run_turn(Prompt::plain(prompt), &mut |event| {
            match event {
                Event::Message { text } => {
                    self.message(id, &text);
                    if !report.is_empty() {
                        report.push_str("\n\n");
                    }
                    report.push_str(&text);
                }
                Event::Streaming { text } => self.progress(id, &text),
                Event::ToolStarted {
                    id: call_id,
                    name,
                    arguments,
                } => self.tool_started(id, &call_id, &name, &arguments),
                Event::ToolFinished {
                    id: call_id,
                    name,
                    summary,
                    result,
                } => self.tool_finished(id, &call_id, &name, &summary, &result),
                Event::ToolChanged {
                    id: call_id,
                    change,
                } => self.tool_changed(id, &call_id, &change),
                Event::Accepted { prompts } => {
                    let accepted = {
                        let mut state = self.0.state.lock().unwrap();
                        state.record(id).map_or_else(Vec::new, |record| {
                            record.pending.iter().take(prompts.len()).cloned().collect()
                        })
                    };
                    for assignment in accepted {
                        self.received(id, &assignment)?;
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
            return;
        }
        let (kind, version, title, details) = {
            let mut state = self.0.state.lock().unwrap();
            let Some(record) = state.record(id) else {
                return;
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
                record.report = Some((version, report));
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
        if let Err(error) = self.receipt(kind, Some(id), version, &title, details) {
            self.persistence_failed(id, error);
        }
        self.0.changed.notify_all();
    }

    pub(super) fn finish_stopped(&self, id: &str) {
        let stopped = {
            let mut state = self.0.state.lock().unwrap();
            state.record(id).and_then(|record| {
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
            && let Err(error) = self.receipt("stopped", Some(id), version, &title, vec![])
        {
            self.persistence_failed(id, error);
        }
        self.0.changed.notify_all();
    }

    fn received(&self, id: &str, assignment: &Assignment) -> Result<(), String> {
        let title = {
            let mut state = self.0.state.lock().unwrap();
            let Some(record) = state.record(id) else {
                return Err("Unknown collaborator".into());
            };
            if record.state != "running" {
                return Err("Collaborator is not running".into());
            }
            record.received_version = assignment.version;
            record.instructions = assignment.instructions.clone();
            record.context = assignment.context.clone();
            record
                .pending
                .retain(|pending| pending.version > assignment.version);
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

    pub(super) fn persistence_failed(&self, id: &str, error: String) {
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

    /// Records the changed paths and their line counts, for receipts that
    /// list changed files with `+` / `−` counts. A multi-file edit keeps each
    /// file's counts under its path.
    pub(super) fn tool_changed(
        &self,
        id: &str,
        call_id: &str,
        change: &crate::file_change::Change,
    ) {
        let counts = |change: &crate::file_change::Change| {
            Value::object([
                ("added", Value::number(change.added)),
                ("removed", Value::number(change.removed)),
                ("created", Value::Bool(change.created)),
            ])
        };
        let mut state = self.0.state.lock().unwrap();
        let Some(record) = state.record(id) else {
            return;
        };
        let Some(Value::Object(tool)) = record
            .tools
            .iter_mut()
            .rev()
            .find(|tool| tool.get("id").and_then(Value::as_str) == Some(call_id))
        else {
            return;
        };
        let mut recorded = counts(change);
        let paths: Vec<String> = if change.files.is_empty() {
            tool.get("arguments")
                .and_then(|arguments| arguments.get("path"))
                .and_then(Value::as_str)
                .map(str::to_owned)
                .into_iter()
                .collect()
        } else {
            change.files.iter().map(|(path, _)| path.clone()).collect()
        };
        if let Value::Object(fields) = &mut recorded
            && !change.files.is_empty()
        {
            let files = change.files.iter().map(|(path, file)| {
                let mut file = counts(file);
                if let Value::Object(fields) = &mut file {
                    fields.insert("path".into(), Value::string(path));
                }
                file
            });
            fields.insert("files".into(), Value::Array(files.collect()));
        }
        tool.insert("change".into(), recorded);
        for path in paths {
            if !record.files.contains(&path) {
                record.files.push(path);
            }
        }
        self.0.changed.notify_all();
    }
}
