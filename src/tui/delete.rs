use super::{
    App, Feedback, Kind,
    drafts::History,
    selector::{Purpose, Selector},
};
use crate::{agent::Agent, sessions::DeleteReport};
use std::thread::{self, JoinHandle};

type Outcome = (Agent, Result<(DeleteReport, bool), String>);
pub(super) struct Job {
    task: JoinHandle<Outcome>,
    id: String,
    title: String,
}

impl App {
    pub(super) fn start_delete(&mut self, id: &str) -> Result<(), String> {
        self.capture_input();
        self.cancel_copy();
        let title = self
            .saved_sessions
            .iter()
            .find(|entry| entry.id == id)
            .map_or(id, |entry| entry.title.as_str())
            .to_string();
        let mut agent = self
            .agent
            .take()
            .ok_or("Session deletion requires a ready turn")?;
        let model = self.config.settings.model.clone();
        let effort = self.config.settings.effort;
        let selected = id.to_string();
        self.deletion_job = Some(Job {
            task: thread::spawn(move || {
                let result = agent.delete_session(&selected, model, effort);
                (agent, result)
            }),
            id: id.into(),
            title,
        });
        if let Some(menu) = &mut self.state.selector
            && let Purpose::Sessions { working, .. } = &mut menu.purpose
        {
            *working = true;
        }
        self.state.status = "Deleting".into();
        self.state.clear_notice();
        self.state.changed();
        Ok(())
    }

    pub(super) fn finish_delete(&mut self, wait: bool) -> Result<bool, String> {
        if self
            .deletion_job
            .as_ref()
            .is_none_or(|job| !wait && !job.task.is_finished())
        {
            return Ok(false);
        }
        let job = self.deletion_job.take().unwrap();
        let (agent, result) = job
            .task
            .join()
            .map_err(|_| "Session deletion worker failed".to_string())?;
        self.persistence = agent.sessions();
        self.archive = agent.archive();
        self.agent = Some(agent);
        match result {
            Ok((report, current)) => {
                if current {
                    self.cancel_copy();
                    self.state.items.clear();
                    self.state.editor = self.state.history.draft(&self.state.editor).clone();
                    self.state.history = History::default();
                    self.edited();
                    self.state.information = None;
                    self.state.activity = None;
                    self.state.generation = self.state.generation.wrapping_add(1);
                    self.pending_command = None;
                    self.save_error = None;
                    self.state.effort = self.agent.as_ref().unwrap().effort().name().into();
                    self.renderer.reset((self.state.width, self.state.height));
                }
                self.saved_sessions.retain(|entry| entry.id != job.id);
                let text = crate::session::delete::message(&job.title, &report, current);
                self.agent.as_ref().unwrap().record_local_details(
                    &format!("/resume · delete {}", job.id),
                    &text,
                    if report.legacy_references > 0 {
                        "warning"
                    } else {
                        "notice"
                    },
                    &[],
                );
                self.state.notify(Feedback::result(
                    if report.legacy_references > 0 {
                        Kind::Warning
                    } else {
                        Kind::Notice
                    },
                    self.archive.redactor.text(&text),
                ));
                if let Err(error) = self.agent.as_ref().unwrap().save_session() {
                    let error = self.archive.redactor.text(&format!(
                        "{text} · Autosave failed: {error}. Session kept in memory."
                    ));
                    self.state.notice = Some(Feedback::result(Kind::Error, error.clone()));
                    self.save_error = Some(error);
                }
            }
            Err(error) => {
                if wait {
                    eprintln!("{}", self.archive.redactor.text(&error));
                }
                self.state.notice = Some(Feedback::result(
                    Kind::Error,
                    self.archive.redactor.text(&error),
                ));
            }
        }
        self.refresh_sessions();
        self.state.status = "Ready".into();
        self.state.changed();
        self.capture_input();
        self.persist_input(true);
        Ok(true)
    }

    fn refresh_sessions(&mut self) {
        let Some(previous) = self.state.selector.take() else {
            return;
        };
        if !matches!(previous.purpose, Purpose::Sessions { .. }) {
            self.state.selector = Some(previous);
            return;
        }
        let current = self.persistence.as_ref().unwrap().id();
        let mut menu = Selector::sessions(&self.saved_sessions, &current);
        menu.searchable |= previous.searchable;
        menu.editor = previous.editor;
        menu.refresh();
        menu.selected = previous.selected.min(menu.filtered.len().saturating_sub(1));
        self.state.selector = Some(menu);
    }
}
