use super::{
    App, Feedback, Kind, Worker,
    activity::Activity,
    jobs::{CatalogPurpose, Job},
    selector::Selector,
    state::Item,
};
use crate::{attachments::Prompt, backend, effort::Effort, session::commands};

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
        if self.state.queue.is_editing() {
            if self.state.editor.prompt().is_empty() {
                self.warn("The draft is empty · Esc cancels, /drafts discards it");
            } else {
                self.finish_draft_edit(true);
            }
            return Ok(false);
        }
        let mut prompt = self.state.editor.prompt();
        if prompt.is_empty() {
            return Ok(false);
        }
        if (self.worker.is_none() || self.state.suggestions.chosen().is_some_and(immediate))
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
        self.state.clear_notice();
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
                self.keep_failed_prompt(prompt);
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
            self.worker = Some(Worker::start(self.agent.take().unwrap(), execution));
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
            match self.agent.as_mut().unwrap().skills() {
                Ok(skills) => {
                    let (result, details) = commands::skills_catalog(
                        skills
                            .into_iter()
                            .map(|skill| (skill.name, skill.description)),
                        self.agent.as_ref().unwrap().skill_warnings(),
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
            self.cancel_copy();
            let result = self.agent.as_mut().unwrap().start_new_selection(
                &self.config.settings.backend_id,
                self.config.settings.model.clone(),
                self.config.settings.effort,
            );
            if let Err(error) = result {
                self.local_start(value);
                self.local_finish(Kind::Error, &error, vec![]);
                return Ok(false);
            }
            let agent = self.agent.as_ref().unwrap();
            self.archive = agent.archive();
            self.persistence = agent.sessions();
            self.state.effort = agent.effort().name().into();
            self.state.clear();
            self.refresh_context();
        }
        self.local_start(value);
        match name.as_str() {
            "/tmp" => match crate::session::temporary::run(self.agent.as_mut().unwrap(), arguments)
            {
                Ok(report) => self.local_finish(Kind::Notice, &report.message, report.details),
                Err(error) => self.local_finish(Kind::Error, &error, vec![]),
            },
            "/checkpoints" => self.start_checkpoints(arguments),
            "/new" if arguments.is_empty() => self.local_finish(
                Kind::Notice,
                "New conversation · saved defaults applied",
                vec![],
            ),
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
            "/backend" => self.backend_command(arguments),
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
            "/effort" => match if arguments.is_empty() {
                Ok(None)
            } else {
                Effort::parse(arguments).map(Some)
            } {
                Ok(Some(Effort::Default)) => {
                    self.apply_selection(self.archive.model.clone(), Effort::Default, false)
                }
                Ok(Some(effort)) if !self.agent.as_ref().unwrap().services().validates_effort() => {
                    self.apply_selection(self.archive.model.clone(), effort, false)
                }
                Ok(effort) => self.load_catalog(CatalogPurpose::Effort {
                    defaults: false,
                    model: self.archive.model.clone(),
                    effort,
                }),
                Err(error) => self.local_finish(Kind::Error, &error, vec![]),
            },
            _ => self.local_finish(Kind::Error, &commands::unknown(&name), vec![]),
        }
        Ok(false)
    }

    pub(super) fn local_start(&mut self, command: &str) {
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
        let command = self
            .pending_command
            .and_then(|index| self.state.items.get(index))
            .and_then(|item| match item {
                Item::Local { command, .. } => command.split_whitespace().next(),
                _ => None,
            })
            .map(str::to_ascii_lowercase);
        let information = match command.as_deref() {
            Some("/tmp") => Some(
                super::information::Information::new("Temporary files".into(), details.clone())
                    .context("this session"),
            ),
            Some("/skills") => Some(
                super::information::Information::new("Skills".into(), details.clone())
                    .aside(&result),
            ),
            Some("/backend") => Some(
                super::information::Information::new("Backends".into(), details.clone())
                    .aside("/backend ID MODEL"),
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
        self.state.status = "Ready".into();
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

    pub(super) fn load_catalog(&mut self, purpose: CatalogPurpose) {
        let home = self
            .config
            .store
            .path()
            .parent()
            .expect("configuration directory");
        let id = match &purpose {
            CatalogPurpose::Models { defaults: true, .. }
            | CatalogPurpose::Effort { defaults: true, .. } => {
                self.config.settings.backend_id.as_str()
            }
            _ => self.agent.as_ref().unwrap().backend_id(),
        };
        let services = if id == self.agent.as_ref().unwrap().backend_id() {
            Ok(self.agent.as_ref().unwrap().services())
        } else {
            self.config.settings.services(home, id)
        };
        match services {
            Ok(services) => self.load_catalog_for(purpose, services),
            Err(error) => self.finish_setting(Kind::Error, &error, vec![]),
        }
    }

    pub(super) fn load_catalog_for(
        &mut self,
        purpose: CatalogPurpose,
        services: backend::Services,
    ) {
        let (title, context, defaults) = match &purpose {
            CatalogPurpose::Models { defaults: true, .. } => ("Default model", "", true),
            CatalogPurpose::Models { .. } => ("Model", "this conversation", false),
            CatalogPurpose::Effort {
                defaults, model, ..
            } => (
                if *defaults {
                    "Default effort"
                } else {
                    "Effort"
                },
                model.as_str(),
                *defaults,
            ),
        };
        let mut menu = Selector::loading(title, "Loading model catalog…", defaults);
        if self.provider_parent.is_some() {
            menu.parent = Some("Provider");
        }
        context.clone_into(&mut menu.context);
        self.state.selector = Some(menu);
        self.state.clear_notice();
        self.catalog_services = Some(services.clone());
        self.job = Some(Job::catalog(services, purpose));
    }

    /// Runs `/copy`, `/drafts` or `/terminal`; returns false for other text.
    fn run_immediate(&mut self, text: &str) -> bool {
        let text = text.trim();
        let (name, arguments) = text
            .split_once(char::is_whitespace)
            .map_or((text, ""), |(name, arguments)| (name, arguments.trim()));
        if name.eq_ignore_ascii_case("/drafts") {
            if arguments.is_empty() {
                self.open_drafts();
            } else {
                self.warn("Usage: /drafts");
            }
        } else if name.eq_ignore_ascii_case("/terminal") {
            self.open_processes(arguments);
        } else {
            return self.copy_command(text);
        }
        true
    }
}

/// Commands that run at once, even while a turn is busy.
pub(super) fn immediate(name: &str) -> bool {
    ["/copy", "/drafts", "/terminal"]
        .iter()
        .any(|command| name.eq_ignore_ascii_case(command))
}
