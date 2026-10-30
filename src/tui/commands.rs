use super::{App, Feedback, Kind, Worker, activity::Activity, jobs::CatalogPurpose, state::Item};
use crate::{attachments::Prompt, effort::Effort, session::commands};

mod catalog;

impl App {
    pub(super) fn help(&mut self) {
        self.local_start("/help");
        self.clear_unless_error();
        self.state.information = Some(super::information::Information::help());
        self.local_finish_quiet(
            Kind::Notice,
            "Commands and controls",
            super::information::help_details(),
        );
    }

    pub(super) fn submit(&mut self) -> Result<bool, String> {
        if !self.imports.is_empty() {
            self.warn("Attachments are still importing. Wait before sending your draft.");
            return Ok(false);
        }
        let mut prompt = self.state.editor.prompt();
        if prompt.is_empty() {
            return Ok(false);
        }
        self.state.sent();
        let turn = self.held.is_some();
        let now = |name: &str| immediate(name) || (turn && menu(name));
        if (self.worker.is_none() || self.state.suggestions.chosen().is_some_and(now))
            && self.state.suggestions.visible
            && prompt.attachments.is_empty()
            && let Some(chosen) = self.state.suggestions.chosen()
        {
            prompt = Prompt::plain(chosen);
        }
        if prompt.attachments.is_empty()
            && prompt.text.split_whitespace().next().is_some_and(immediate)
        {
            self.state.history.submitted(&mut self.state.editor);
            self.edited();
            self.run_immediate(&prompt.text);
            self.capture_input();
            return Ok(false);
        }
        if turn
            && prompt.attachments.is_empty()
            && prompt.text.split_whitespace().next().is_some_and(menu)
        {
            // A menu opens over the running turn; what it changes waits for it.
            self.state.history.submitted(&mut self.state.editor);
            self.edited();
            self.capture_input();
            return self.dispatch(prompt);
        }
        if self.busy() {
            if let Err(error) = self.state.queue.push(prompt.clone()) {
                self.warn(&error);
                return Ok(false);
            }
            self.state.history.submitted(&mut self.state.editor);
            self.edited();
            // Saving the input records the message, which lets the agent accept it.
            self.capture_input();
            return Ok(false);
        }
        self.state.history.submitted(&mut self.state.editor);
        self.edited();
        self.capture_input();
        self.dispatch(prompt)
    }

    pub(super) fn dispatch(&mut self, prompt: Prompt) -> Result<bool, String> {
        let command = prompt.attachments.is_empty() && prompt.text.trim_start().starts_with('/');
        let init = command
            && commands::split(&prompt.text).is_some_and(|(name, arguments)| {
                name.eq_ignore_ascii_case("/init") && arguments.is_empty()
            });
        if command && self.run_immediate(&prompt.text) {
            return Ok(false);
        }
        self.capture_input();
        // Automatic dispatch keeps unread errors; only a send clears them.
        self.state.clear_notice();
        self.state.ending = None;
        self.state.information = None;
        if !command || init {
            let input = self.agent.as_ref().unwrap().input_checkpoint();
            let execution = if init {
                Prompt::plain(commands::INIT_PROMPT)
            } else {
                prompt.clone()
            };
            if let Err(error) = self.agent.as_mut().unwrap().prepare_turn(&execution) {
                self.state.message(Kind::Error, &error);
                self.restore_failed_start(prompt);
                return Ok(false);
            }
            self.state.history.record(&prompt);
            self.capture_input();
            self.refresh_context();
            self.state.begin_tool_turn();
            self.renderer.scroll(super::viewport::Scroll::End);
            let mut submission = super::recovery::Submission::new(input);
            submission.accepted(prompt.clone(), self.state.items.len());
            self.submission = Some(submission);
            self.state
                .message(Kind::User, &self.archive.redactor.text(&prompt.display()));
            self.state.activity = Some(Activity::new());
            self.state.backend_failure = None;
            self.process_observer = self.agent.as_ref().unwrap().processes();
            self.collaboration_auto_resume = true;
            self.collaboration_return_pending = false;
            let worker = Worker::start(self.hold(), execution);
            self.run_worker(worker);
            self.state.status = "Starting...".into();
            return Ok(false);
        }
        let value = prompt.text.trim();
        let (name, arguments) = commands::split(value).unwrap();
        let name = commands::canonical(name).to_ascii_lowercase();
        if name == "/exit" && arguments.is_empty() {
            return Ok(true);
        }
        if name == "/help" && arguments.is_empty() {
            self.help();
            return Ok(false);
        }
        if name == "/skills" && arguments.is_empty() {
            self.local_start("/skills");
            let listing = if let Some(agent) = &mut self.agent {
                agent
                    .skills()
                    .map(|skills| (skills, agent.skill_warnings().to_vec()))
            } else {
                // A turn holds the agent; read the same library directly.
                let mut library = crate::skills::Library::at(self.facts().skills.clone());
                library.refresh();
                Ok((library.list(), library.warnings().to_vec()))
            };
            match listing {
                Ok((skills, warnings)) => {
                    let (result, details) = commands::skills_catalog(
                        skills
                            .into_iter()
                            .map(|skill| (skill.name, skill.description)),
                        &warnings,
                    );
                    self.local_finish(Kind::Notice, &result, details);
                }
                Err(error) => self.local_finish(Kind::Error, &error, vec![]),
            }
            return Ok(false);
        }
        if name == "/resume" {
            self.capture_input();
            if arguments.is_empty() {
                self.open_sessions();
            } else {
                self.resume_session(arguments);
            }
            return Ok(false);
        }
        if name == "/new" && arguments.is_empty() {
            self.new_conversation();
            return Ok(false);
        }
        self.local_start(value);
        match name.as_str() {
            "/tmp" => match crate::session::temporary::run(self.agent.as_mut().unwrap(), arguments)
            {
                Ok(report) => self.local_finish(Kind::Notice, &report.message, report.details),
                Err(error) => self.local_finish(Kind::Error, &error, vec![]),
            },
            "/checkpoints" => self.start_checkpoints(arguments),
            "/export" if arguments.is_empty() => match self.archive.save() {
                Ok(path) => self.local_finish(
                    Kind::Notice,
                    &format!(
                        "Saved {}",
                        path.file_name().unwrap_or_default().to_string_lossy()
                    ),
                    vec![],
                ),
                Err(error) => self.local_finish(Kind::Error, &error, vec![]),
            },
            "/settings" if arguments.is_empty() => {
                self.open_settings();
            }
            "/routing" if arguments.is_empty() => self.open_routing(false),
            "/model" => {
                if !arguments.is_empty()
                    && let Err(error) = crate::config::validate_model(arguments)
                {
                    self.local_finish(Kind::Error, &error, vec![]);
                    return Ok(false);
                }
                if arguments.is_empty() {
                    self.open_model_providers(false);
                } else {
                    self.load_catalog(CatalogPurpose::Models {
                        defaults: false,
                        id: Some(arguments.into()),
                    });
                }
            }
            "/effort" if !self.facts().has_model => self.local_finish(
                Kind::Warning,
                "No model selected · Choose a model with /model before setting effort",
                vec![],
            ),
            "/effort" => match if arguments.is_empty() {
                Ok(None)
            } else {
                Effort::parse(arguments).map(Some)
            } {
                Ok(Some(Effort::Default)) => {
                    let model = self.facts().model.clone();
                    self.apply_selection(model, Effort::Default, false)
                }
                Ok(Some(effort)) if !self.facts().services.validates_effort() => {
                    let model = self.facts().model.clone();
                    self.apply_selection(model, effort, false)
                }
                Ok(effort) => {
                    let model = self.facts().model.clone();
                    self.load_catalog(CatalogPurpose::Effort {
                        defaults: false,
                        model,
                        effort,
                    })
                }
                Err(error) => self.local_finish(Kind::Error, &error, vec![]),
            },
            _ => self.local_finish(Kind::Error, &commands::unknown(&name), vec![]),
        }
        Ok(false)
    }

    /// Opens a command row. A menu used during a turn reports through notices
    /// instead, so the running turn keeps its rows.
    pub(super) fn local_start(&mut self, command: &str) {
        self.local_command = command
            .split_whitespace()
            .next()
            .map(str::to_ascii_lowercase);
        if self.held.is_some() {
            return;
        }
        self.state.close_tools();
        self.state.changed();
        self.pending_command = Some(self.state.items.len());
        self.state.items.push(Item::Local {
            command: command.into(),
            result: None,
            kind: Kind::Notice,
            details: vec![],
        });
    }

    pub(super) fn local_finish(
        &mut self,
        kind: Kind,
        result: &str,
        details: Vec<(String, String)>,
    ) {
        self.local_finish_with_change(kind, result, details, false);
    }

    pub(super) fn local_finish_change(
        &mut self,
        kind: Kind,
        result: &str,
        details: Vec<(String, String)>,
    ) {
        self.local_finish_with_change(kind, result, details, true);
    }

    fn local_finish_with_change(
        &mut self,
        kind: Kind,
        result: &str,
        details: Vec<(String, String)>,
        change: bool,
    ) {
        let result = self.archive.redactor.text(result);
        let details: Vec<_> = details
            .into_iter()
            .map(|(key, value)| {
                (
                    self.archive.redactor.text(&key),
                    self.archive.redactor.text(&value),
                )
            })
            .collect();
        let information = match self.local_command.as_deref() {
            Some("/tmp") => Some(
                super::information::Information::new("Temporary files".into(), details.clone())
                    .context("this session"),
            ),
            Some("/skills") => Some(
                super::information::Information::new("Skills".into(), details.clone())
                    .aside(&result),
            ),
            Some("/checkpoints") => Some(super::information::Information::new(
                result.clone(),
                details.clone(),
            )),
            _ => None,
        };
        match information {
            Some(information) if matches!(kind, Kind::Notice) && !details.is_empty() => {
                self.clear_unless_error();
                self.state.information = Some(information);
            }
            _ => {
                let mut feedback = Feedback::result(kind, result.clone());
                if change {
                    feedback = feedback.change();
                }
                self.state.notify(feedback);
            }
        }
        self.local_finish_quiet(kind, &result, details);
    }

    /// An information panel replaces the notice unless it reports an error.
    fn clear_unless_error(&mut self) {
        if self
            .state
            .notice
            .as_ref()
            .is_none_or(|notice| !matches!(notice.kind, Kind::Error))
        {
            self.state.notice = None;
        }
    }

    pub(super) fn local_finish_quiet(
        &mut self,
        kind: Kind,
        result: &str,
        details: Vec<(String, String)>,
    ) {
        let result = self.archive.redactor.text(result);
        self.local_command = None;
        if let Some(index) = self.pending_command.take()
            && let Some(Item::Local {
                command,
                result: value,
                kind: current,
                details: rows,
            }) = self.state.items.get_mut(index)
        {
            if let Some(agent) = &self.agent {
                agent.record_local_details(
                    command,
                    &result,
                    match kind {
                        Kind::Error => "error",
                        Kind::Warning => "warning",
                        _ => "notice",
                    },
                    &details,
                );
            }
            *value = Some(result);
            *current = kind;
            *rows = details;
        }
        self.state.changed();
        if self.held.is_none() {
            self.state.status = "Ready".into();
        }
        if let Some(agent) = &self.agent
            && let Err(error) = agent.save_session()
        {
            let error = agent.redact(&format!(
                "Autosave failed: {error}. Session kept in memory."
            ));
            self.state
                .notify(Feedback::result(Kind::Error, error.clone()));
            self.save_error = Some(error);
        }
    }

    /// Runs `/copy`, `/terminal` or another panel command; returns false for other text.
    fn run_immediate(&mut self, text: &str) -> bool {
        let text = text.trim();
        let (name, arguments) = text
            .split_once(char::is_whitespace)
            .map_or((text, ""), |(name, arguments)| (name, arguments.trim()));
        if name.eq_ignore_ascii_case("/terminal") {
            self.open_processes(arguments);
        } else if name.eq_ignore_ascii_case("/agents") {
            if arguments.is_empty() {
                self.open_agents();
            } else {
                self.warn("Usage: /agents");
            }
        } else if name.eq_ignore_ascii_case("/ask") {
            if arguments.is_empty() {
                self.open_ask();
            } else {
                self.warn("Usage: /ask");
            }
        } else if name.eq_ignore_ascii_case("/permissions") {
            self.open_permissions(arguments);
        } else if name.eq_ignore_ascii_case("/todo") {
            if arguments.is_empty() {
                self.open_todo();
            } else {
                self.warn("Usage: /todo");
            }
        } else {
            return self.copy_command(text);
        }
        true
    }
}

/// Commands that run at once, even while a turn is busy.
pub(super) fn immediate(name: &str) -> bool {
    [
        "/copy",
        "/terminal",
        "/agents",
        "/ask",
        "/todo",
        "/permissions",
    ]
    .iter()
    .any(|command| name.eq_ignore_ascii_case(command))
}

/// Commands that open a menu over a running turn instead of waiting for it.
/// Commands that start work or replace the conversation still wait.
fn menu(name: &str) -> bool {
    let name = commands::canonical(name);
    [
        "/settings",
        "/model",
        "/effort",
        "/routing",
        "/help",
        "/skills",
    ]
    .iter()
    .any(|command| name.eq_ignore_ascii_case(command))
}
