use super::{
    App, Feedback, Kind,
    editor::Editor,
    selector::Selector,
    state::{Item, Recent},
};
use crate::{
    events::Event,
    sessions::{Input, Record},
};

impl App {
    /// A plain-chat staging list becomes visible composer elements when the
    /// session opens in the TUI. Clear staging only after the editor owns it.
    pub(super) fn restore_staged(&mut self) {
        let Some(handle) = self.persistence.clone() else {
            return;
        };
        let mut input = handle.snapshot().input;
        if input.staged.is_empty() {
            return;
        }
        let empty = self.state.editor.text.is_empty();
        let cursor = self.state.editor.cursor;
        self.state.editor.cursor = self.state.editor.text.len();
        for attachment in input.staged.drain(..) {
            self.state.editor.attach(attachment);
            self.state.editor.insert(" ");
        }
        self.state.editor.cursor = if empty {
            self.state.editor.text.len()
        } else {
            cursor
        };
        self.state.history.edited();
        self.edited();
        handle.input(input);
        self.persist_input(true);
    }

    pub(super) fn recent_sessions(&mut self) {
        let Some(handle) = &self.persistence else {
            return;
        };
        match handle.store().list() {
            Ok(listing) => {
                // Unreadable sessions outrank the hint, so the warning shows.
                if !listing.warnings.is_empty() {
                    let count = listing.warnings.len();
                    self.state.notify(
                        Feedback::result(
                            Kind::Warning,
                            format!(
                                "{count} saved {} could not be read",
                                if count == 1 { "session" } else { "sessions" }
                            ),
                        )
                        .action("/resume", "details"),
                    );
                }
                self.offer_recent(&listing.sessions);
            }
            Err(error) => self.warn(&error),
        }
    }

    /// The startup space offers other conversations until the first prompt.
    pub(super) fn offer_recent(&mut self, sessions: &[crate::sessions::Summary]) {
        let Some(handle) = &self.persistence else {
            return;
        };
        self.state.opening.recent = sessions
            .iter()
            .filter(|session| session.id != handle.id())
            .take(3)
            .map(|session| Recent {
                title: self.archive.redactor.text(&session.title),
                age: session.elapsed(),
            })
            .collect();
    }

    pub(super) fn capture_input(&self) {
        if self.deletion_job.is_some() {
            return;
        }
        let draft = super::drafts::draft;
        let editor = &self.state.editor;
        // Capturing also lets the agent accept the queued prompts, so it runs
        // with or without session storage.
        self.state.queue.capture(editor, |queued, mut paused| {
            let Some(handle) = &self.persistence else {
                return;
            };
            if self.state.history.has_edited_recall() {
                paused.push(draft(self.state.queue.draft(editor)));
            }
            handle.capture_input(Input {
                draft: draft(self.state.history.draft(self.state.queue.draft(editor))),
                staged: vec![],
                queued,
                paused,
                previous: None,
                history: self.state.history.snapshot(),
                question: self.state.ask_draft.clone(),
            });
        });
    }

    pub(super) fn persist_input(&mut self, immediate: bool) -> bool {
        if self.deletion_job.is_some() {
            return false;
        }
        if immediate {
            self.capture_input();
        }
        let Some(handle) = &self.persistence else {
            return false;
        };
        let result = if immediate {
            handle.flush().map(|()| true)
        } else {
            handle.flush_due_background()
        };
        match result {
            Err(error) => {
                let error = self.archive.redactor.text(&format!(
                    "Autosave failed: {error}. Session kept in memory."
                ));
                if self.save_error.as_ref() != Some(&error) {
                    self.state
                        .notify(Feedback::result(Kind::Error, error.clone()));
                    self.save_error = Some(error);
                    return true;
                }
            }
            Ok(true) => {
                self.save_error = None;
            }
            Ok(false) => {}
        }
        false
    }

    pub(super) fn open_sessions(&mut self) {
        self.state.clear_notice();
        self.state.information = None;
        let result = crate::session::delete::sessions(self.agent.as_ref().unwrap());
        match result {
            Ok((sessions, warnings)) => {
                let current = self.persistence.as_ref().unwrap().id();
                self.saved_sessions = sessions;
                if !warnings.is_empty() {
                    self.state.message(Kind::Warning, &warnings.join(" · "));
                }
                self.state.selector = Some(Selector::sessions(&self.saved_sessions, &current));
            }
            Err(error) => {
                self.local_start("/resume");
                self.local_finish(Kind::Error, &error, vec![]);
            }
        }
    }

    pub(super) fn resume_session(&mut self, id: &str) {
        self.cancel_copy();
        self.capture_input();
        let result = self.agent.as_mut().unwrap().resume(id);
        match result {
            Ok(status) => {
                let agent = self.agent.as_ref().unwrap();
                self.archive = agent.archive();
                self.todo_message_len = usize::MAX;
                self.persistence = agent.sessions();
                let document = self.persistence.as_ref().unwrap().snapshot();
                let title = document.title();
                if self.state.items.is_empty() {
                    self.state.generation = self.state.generation.wrapping_add(1);
                    self.state.changed();
                } else {
                    self.state.clear();
                }
                self.pending_command = None;
                self.state.selector = None;
                self.state.activity = None;
                self.state.information = None;
                self.state.clear_notice();
                self.state.queue.reset(
                    document
                        .input
                        .paused
                        .iter()
                        .map(|draft| {
                            let mut editor = Editor::default();
                            editor.set(draft.prompt());
                            editor.cursor = draft.cursor;
                            editor
                        })
                        .collect(),
                );
                self.state.effort = document.effort.name().into();
                self.collaboration_seen.clear();
                self.collaboration_receipt_calls.clear();
                self.collaboration_return_pending = false;
                self.collaboration_auto_resume = true;
                let mut turn_recorded = false;
                for record in document.records() {
                    match record {
                        Record::TurnStart => {
                            self.state.begin_tool_turn();
                            turn_recorded = true;
                        }
                        Record::Text { role, text } => match role.as_str() {
                            "user" if text == crate::session::commands::INIT_PROMPT => {
                                self.state.message(Kind::User, "/init");
                            }
                            "user" => self.state.message(Kind::User, &text),
                            "assistant" => self.state.message(Kind::Assistant, &text),
                            _ => {}
                        },
                        Record::Question { request } => {
                            if !self.state.items.iter().any(|item| {
                                matches!(item,
                                Item::Tool { id, name, result: None, .. }
                                if id == &request.id && name == "ask_user")
                            }) {
                                self.state.event(Event::ToolStarted {
                                    arguments: request.arguments(),
                                    id: request.id,
                                    name: "ask_user".into(),
                                });
                            }
                        }
                        Record::Tool {
                            id,
                            name,
                            arguments,
                            summary,
                            result,
                            change,
                        } => {
                            if name == "agent" && super::agents::receipt_replaces_tool(&arguments) {
                                if result
                                    .get("error")
                                    .and_then(crate::json::Value::as_str)
                                    .is_some()
                                {
                                    self.state.diagnostic(&self.archive.redactor.text(&summary));
                                }
                                continue;
                            }
                            if !self.state.items.iter().any(|item| {
                                matches!(item,
                                Item::Tool { id: existing, name: tool, result: None, .. }
                                if existing == &id && tool == &name)
                            }) {
                                self.state.event(Event::ToolStarted {
                                    id: id.clone(),
                                    name: name.clone(),
                                    arguments,
                                });
                            }
                            if let Some(change) = change {
                                self.state.event(Event::ToolChanged {
                                    id: id.clone(),
                                    change,
                                });
                            }
                            self.state.event(Event::ToolFinished {
                                id,
                                name,
                                summary,
                                result,
                            });
                        }
                        Record::Local {
                            command,
                            result,
                            kind,
                            details,
                        } => {
                            self.state.close_tools();
                            self.state.items.push(Item::Local {
                                command,
                                result: Some(result),
                                details,
                                kind: match kind.as_str() {
                                    "error" => Kind::Error,
                                    "warning" => Kind::Warning,
                                    _ => Kind::Notice,
                                },
                            });
                        }
                        Record::Collaboration { event } => {
                            let sequence = super::agents::number(&event, "sequence");
                            if self.collaboration_seen.insert(sequence) {
                                self.state
                                    .collaboration_event(self.archive.redactor.value(&event));
                            }
                        }
                    }
                }
                self.state.close_tools();
                self.collaboration_observer = agent.collaboration();
                self.question_observer = agent.questions();
                self.question_observer.set_available(true);
                self.collaboration_cursor = self.collaboration_observer.events_since(0).0;
                self.state.collaboration = self
                    .collaboration_observer
                    .snapshot()
                    .into_iter()
                    .map(|record| self.archive.redactor.value(&record))
                    .collect();
                self.collaboration_return_pending = agent.has_open_collaboration_report();
                // Saved records do not contain execution timings.
                if turn_recorded {
                    self.state.finish_tool_turn();
                }
                for item in &mut self.state.items {
                    if let Item::Tool { presentation, .. } = item {
                        presentation.started = None;
                        presentation.elapsed = None;
                    }
                }
                let cursor = document.input.draft.cursor;
                self.state.editor.set(document.input.draft.prompt());
                self.state.editor.cursor = cursor;
                self.state.history.restore(document.input.history);
                self.state.suggestions.refresh(&self.state.editor.text);
                let pending_question = agent.has_pending_question();
                self.restore_staged();
                self.refresh_question();
                self.refresh_todo();
                if pending_question {
                    self.state.begin_tool_turn();
                    self.state.activity = Some(super::activity::Activity::new());
                    self.worker =
                        Some(super::Worker::continue_question(self.agent.take().unwrap()));
                }
                self.local_start(&format!("/resume {id}"));
                self.local_finish(Kind::Notice, &format!("Resumed {title} · {status}"), vec![]);
                self.refresh_context();
                self.state.changed();
            }
            Err(error) => {
                self.state.selector = None;
                self.local_start(&format!("/resume {id}"));
                self.local_finish(Kind::Error, &error, vec![]);
            }
        }
    }
}
