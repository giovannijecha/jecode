use super::*;

impl Observer {
    pub(crate) fn update(
        &self,
        id: &str,
        instructions: Option<String>,
        context: Option<String>,
    ) -> Result<Value, String> {
        if instructions.is_none() && context.is_none() {
            return Err("update requires instructions or context".into());
        }
        let (version, received_version, wake) = {
            let mut state = self.0.state.lock().unwrap();
            let index = state
                .records
                .iter()
                .position(|record| record.id == id)
                .ok_or("Unknown collaborator")?;
            let worker = state
                .workers
                .iter()
                .find(|worker| worker.id == id)
                .ok_or("Collaborator worker is unavailable")?;
            let inbox = worker.inbox.clone();
            let wake = worker.wake.clone();
            let record = &mut state.records[index];
            if matches!(record.state, "stopping" | "stopped" | "unknown") {
                return Err("Collaborator is not available".into());
            }
            // A partial update builds on the newest sent revision, even if
            // the worker has not received it yet.
            let latest = record.pending.back();
            let instructions = instructions.unwrap_or_else(|| {
                latest
                    .map_or(&record.instructions, |a| &a.instructions)
                    .clone()
            });
            let context =
                context.unwrap_or_else(|| latest.map_or(&record.context, |a| &a.context).clone());
            let running = record.state == "running";
            let old_state = record.state;
            let old_reviewed = record.reviewed;
            record.state = "running";
            record.reviewed = false;
            record.sent_version += 1;
            let version = record.sent_version;
            let received_version = record.received_version;
            record.pending.push_back(Assignment {
                version,
                instructions: instructions.clone(),
                context: context.clone(),
            });
            let title = record.title.clone();
            if let Err(error) = self.receipt_locked(
                &mut state,
                "sent",
                Some(id),
                version,
                &title,
                vec![instructions.clone()],
            ) {
                let record = &mut state.records[index];
                record
                    .pending
                    .retain(|assignment| assignment.version != version);
                record.sent_version = version - 1;
                record.state = old_state;
                record.reviewed = old_reviewed;
                return Err(error);
            }
            if running {
                // The worker can only select an initial assignment while holding
                // this same state lock. Its handoff drains the inbox atomically.
                let queued = inbox.with(|entries| {
                    if entries
                        .push(Prompt::plain(format!(
                            "Assignment update v{version}: {instructions}\n\n{context}"
                        )))
                        .is_ok()
                    {
                        entries.record();
                        true
                    } else {
                        false
                    }
                });
                if !queued {
                    state.records[index].activity = "Update queued for follow-up".into();
                }
            }
            (version, received_version, wake)
        };
        let _ = wake.send(());
        Ok(Value::object([
            ("id", Value::string(id)),
            ("sent_version", Value::number(version)),
            ("received_version", Value::number(received_version)),
        ]))
    }

    pub(crate) fn receipt(
        &self,
        kind: &str,
        id: Option<&str>,
        version: usize,
        title: &str,
        details: Vec<String>,
    ) -> Result<Value, String> {
        let mut state = self.0.state.lock().unwrap();
        self.receipt_locked(&mut state, kind, id, version, title, details)
    }

    fn receipt_locked(
        &self,
        state: &mut State,
        kind: &str,
        id: Option<&str>,
        version: usize,
        title: &str,
        details: Vec<String>,
    ) -> Result<Value, String> {
        let parent = self
            .0
            .parent
            .lock()
            .unwrap()
            .clone()
            .ok_or("Session storage is unavailable")?;
        let mut event = Value::object([
            ("type", Value::string("collaboration")),
            ("sequence", Value::number(state.receipts.len() + 1)),
            ("kind", Value::string(kind)),
            ("id", id.map_or(Value::Null, Value::string)),
            (
                "number",
                Value::number(
                    id.and_then(|id| {
                        state
                            .records
                            .iter()
                            .find(|record| record.id == id)
                            .map(|record| record.number)
                    })
                    .unwrap_or(0),
                ),
            ),
            ("title", Value::string(title)),
            ("version", Value::number(version)),
            (
                "summary",
                Value::string(format!("Agent {} {kind}", id.unwrap_or("group"))),
            ),
            (
                "details",
                Value::Array(details.iter().map(Value::string).collect()),
            ),
            (
                "records",
                Value::Array(state.records.iter().map(|record| record.value()).collect()),
            ),
            (
                "after_message",
                Value::number(parent.0.messages.lock().unwrap().len()),
            ),
        ]);
        self.place(state, &mut event);
        event = parent.0.redactor.value(&event);
        if let Some(handle) = &parent.1 {
            event = handle.append_event(&parent.0, event)?;
        } else {
            parent.0.events.lock().unwrap().push(event.clone());
        }
        state.receipts.push(event.clone());
        self.0.changed.notify_all();
        Ok(event)
    }

    pub(crate) fn wait(
        &self,
        id: Option<&str>,
        timeout: Duration,
        cancellation: &crate::cancel::Cancellation,
    ) -> Result<Vec<Value>, String> {
        let mut state = self.0.state.lock().unwrap();
        if id.is_some_and(|id| !state.records.iter().any(|record| record.id == id)) {
            return Err("Unknown collaborator".into());
        }
        let started = std::time::Instant::now();
        loop {
            if cancellation.requested() {
                return Err("Operation cancelled".into());
            }
            if !state.records.iter().any(|r| {
                id.is_none_or(|id| r.id == id) && matches!(r.state, "running" | "stopping")
            }) {
                break;
            }
            let remaining = timeout.saturating_sub(started.elapsed());
            if remaining.is_zero() {
                break;
            }
            state = self
                .0
                .changed
                .wait_timeout(state, remaining.min(Duration::from_millis(50)))
                .unwrap()
                .0;
        }
        Ok(state
            .records
            .iter()
            .filter(|r| id.is_none_or(|id| r.id == id))
            .map(|record| record.value())
            .collect())
    }

    pub(crate) fn stop(&self, id: &str) -> Result<Value, String> {
        let title = {
            let mut state = self.0.state.lock().unwrap();
            if !state.records.iter().any(|record| record.id == id) {
                return Err("Unknown collaborator".into());
            }
            if !state.workers.iter().any(|worker| worker.id == id) {
                return Err("Collaborator worker is unavailable".into());
            }
            let record = state
                .records
                .iter_mut()
                .find(|r| r.id == id)
                .ok_or("Unknown collaborator")?;
            if matches!(record.state, "stopped" | "unknown") {
                return Err("Collaborator is no longer running".into());
            }
            record.state = "stopping";
            let title = record.title.clone();
            let worker = state
                .workers
                .iter()
                .find(|w| w.id == id)
                .ok_or("Collaborator worker is unavailable")?;
            worker.cancellation.cancel();
            let _ = worker.wake.send(());
            title
        };
        self.receipt("sent", Some(id), 0, &title, vec!["Stop requested".into()])
    }

    pub(crate) fn review(
        &self,
        id: &str,
        report_version: usize,
        note: String,
    ) -> Result<Value, String> {
        let (title, previous) = {
            let mut state = self.0.state.lock().unwrap();
            let record = state
                .records
                .iter_mut()
                .find(|r| r.id == id)
                .ok_or("Unknown collaborator")?;
            if record.report.as_ref().map(|(version, _)| *version) != Some(report_version)
                || report_version != record.sent_version
                || !matches!(record.state, "returned" | "failed")
            {
                return Err("Report version is not the latest returned assignment".into());
            }
            let previous = record.reviewed;
            record.reviewed = true;
            (record.title.clone(), previous)
        };
        match self.receipt("reviewed", Some(id), report_version, &title, vec![note]) {
            Ok(value) => Ok(value),
            Err(error) => {
                if let Some(record) = self
                    .0
                    .state
                    .lock()
                    .unwrap()
                    .records
                    .iter_mut()
                    .find(|record| record.id == id)
                {
                    record.reviewed = previous;
                }
                Err(error)
            }
        }
    }

    pub(crate) fn workspace(&self, id: &str) -> Result<Workspace, String> {
        let state = self.0.state.lock().unwrap();
        let record = state
            .records
            .iter()
            .find(|r| r.id == id)
            .ok_or("Unknown collaborator")?;
        record.workspace.clone().ok_or_else(|| {
            if record.workspace_closed {
                "Collaborator workspace is closed".into()
            } else {
                record
                    .error
                    .clone()
                    .unwrap_or_else(|| "Collaborator workspace is unavailable".into())
            }
        })
    }

    pub(crate) fn close(&self, id: &str) -> Result<Value, String> {
        let already_closed = {
            let state = self.0.state.lock().unwrap();
            state
                .records
                .iter()
                .find(|record| record.id == id)
                .ok_or("Unknown collaborator")?
                .workspace_closed
        };
        if already_closed {
            return self.finish_close(id, Value::object([("closed", Value::Bool(true))]));
        }
        let missing_after_restart = {
            let state = self.0.state.lock().unwrap();
            let record = state.records.iter().find(|record| record.id == id).unwrap();
            record.workspace.is_none() && !state.workers.iter().any(|worker| worker.id == id)
        };
        if missing_after_restart {
            let managed = self
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
            let info = {
                let state = self.0.state.lock().unwrap();
                state
                    .records
                    .iter()
                    .find(|record| record.id == id)
                    .unwrap()
                    .workspace_info
                    .clone()
            };
            if !Workspace::already_removed(&info, id, &managed)? {
                return Err("Collaborator workspace is unavailable; owned files may remain".into());
            }
            {
                let mut state = self.0.state.lock().unwrap();
                let record = state
                    .records
                    .iter_mut()
                    .find(|record| record.id == id)
                    .unwrap();
                record.state = "stopped";
                record.workspace_closed = true;
            }
            return self.finish_close(id, Value::object([("closed", Value::Bool(true))]));
        }
        let worker = {
            let mut state = self.0.state.lock().unwrap();
            let index = state.workers.iter().position(|w| w.id == id);
            let record = state
                .records
                .iter_mut()
                .find(|r| r.id == id)
                .ok_or("Unknown collaborator")?;
            if index.is_some() && record.state == "running" {
                record.state = "stopping";
            }
            index.map(|index| {
                let worker = state.workers.remove(index);
                worker.cancellation.cancel();
                let _ = worker.wake.send(());
                worker
            })
        };
        if let Some(task) = worker.and_then(|worker| worker.task) {
            let _ = task.join();
        }
        let workspace = self.workspace(id)?;
        let result = workspace.close()?;
        {
            let mut state = self.0.state.lock().unwrap();
            let record = state.record(id).unwrap();
            record.state = "stopped";
            record.workspace_closed = true;
            record.workspace = None;
        }
        self.finish_close(id, result)
    }

    fn finish_close(&self, id: &str, result: Value) -> Result<Value, String> {
        let (title, version) = {
            let state = self.0.state.lock().unwrap();
            let record = state
                .records
                .iter()
                .find(|record| record.id == id)
                .ok_or("Unknown collaborator")?;
            (record.title.clone(), record.received_version)
        };
        self.receipt("closed", Some(id), version, &title, vec![])?;
        self.0
            .state
            .lock()
            .unwrap()
            .records
            .retain(|record| record.id != id);
        self.0.changed.notify_all();
        Ok(result)
    }
}
