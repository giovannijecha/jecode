use super::{App, Feedback, Kind, drafts::Queue, editor::Editor, selector::Selector, state::Item};
use crate::{
    events::Event,
    sessions::{Input, Record},
};

impl App {
    pub(super) fn session_hint(&mut self) {
        let Some(handle) = &self.persistence else {
            return;
        };
        match handle.store().list() {
            Ok(listing) => {
                if let Some(session) = listing
                    .sessions
                    .iter()
                    .find(|session| session.id != handle.id())
                {
                    self.state.notice = Some(Feedback::result(
                        Kind::Notice,
                        format!("Previous session: {} · /resume", session.title),
                    ));
                }
                if !listing.warnings.is_empty() {
                    self.warn(&format!(
                        "{} saved session(s) could not be read · /resume for details",
                        listing.warnings.len()
                    ));
                }
            }
            Err(error) => self.warn(&error),
        }
    }

    pub(super) fn capture_input(&self) {
        if self.deletion_job.is_some() {
            return;
        }
        if let Some(handle) = &self.persistence {
            let draft = super::drafts::draft;
            let (queued, mut paused) = self.state.queue.snapshot(&self.state.editor);
            if self.state.history.has_edited_recall() {
                paused.push(draft(self.state.queue.draft(&self.state.editor)));
            }
            handle.input(Input {
                draft: draft(
                    self.state
                        .history
                        .draft(self.state.queue.draft(&self.state.editor)),
                ),
                queued,
                paused,
                previous: None,
                history: self.state.history.snapshot(),
            });
        }
    }

    pub(super) fn persist_input(&mut self, immediate: bool) -> bool {
        if self.deletion_job.is_some() {
            return false;
        }
        self.capture_input();
        let Some(handle) = &self.persistence else {
            return false;
        };
        let result = if immediate {
            handle.flush().map(|()| true)
        } else {
            handle.flush_due()
        };
        match result {
            Err(error) => {
                let error = self.archive.redactor.text(&format!(
                    "Autosave failed: {error}. Session kept in memory."
                ));
                if self.save_error.as_ref() != Some(&error) {
                    self.state.notice = Some(Feedback::result(Kind::Error, error.clone()));
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
                for warning in warnings {
                    self.state.message(Kind::Warning, &warning);
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
                self.state.queue = Queue::default();
                self.state.queue.paused = document
                    .input
                    .paused
                    .iter()
                    .map(|draft| {
                        let mut editor = Editor::default();
                        editor.set(draft.prompt());
                        editor.cursor = draft.cursor;
                        editor
                    })
                    .collect();
                self.state.effort = document.effort.name().into();
                for record in document.records() {
                    match record {
                        Record::Text { role, text } => match role.as_str() {
                            "user" => self.state.message(Kind::User, &text),
                            "assistant" => self.state.message(Kind::Assistant, &text),
                            _ => {}
                        },
                        Record::Tool {
                            id,
                            name,
                            arguments,
                            summary,
                            result,
                        } => {
                            self.state.event(Event::ToolStarted {
                                id: id.clone(),
                                name: name.clone(),
                                arguments,
                            });
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
                    }
                }
                self.state.close_tools();
                // Saved records do not contain execution timings.
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
                self.local_start(&format!("/resume {id}"));
                self.local_finish(Kind::Notice, &format!("Resumed {title} · {status}"), vec![]);
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
