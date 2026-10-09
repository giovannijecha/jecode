use super::{
    App, Feedback, Kind, Worker,
    activity::Activity,
    jobs::{CatalogPurpose, Job},
    selector::Selector,
    state::Item,
};
use crate::{
    attachments::Prompt,
    effort::Effort,
    session::commands::{self, COMMANDS},
};

impl App {
    pub(super) fn help(&mut self) {
        self.local_start("/help");
        let mut details: Vec<(String, String)> = COMMANDS
            .iter()
            .map(|command| (command.name.into(), command.description.into()))
            .collect();
        details.extend([
            ("Enter".into(), "Send · queue while working".into()),
            (
                "Ctrl+J".into(),
                "New line · Shift/Alt+Enter when supported".into(),
            ),
            (
                "Alt+Up · /drafts".into(),
                "Open pending drafts · edit or discard one message".into(),
            ),
            (
                "Ctrl+P/N".into(),
                "Sent prompt history · commands are excluded".into(),
            ),
            (
                "PgUp/PgDn · mouse wheel".into(),
                "Scroll conversation · new output keeps your reading position".into(),
            ),
            (
                "Alt+Home/End".into(),
                "Conversation start / follow latest output · Esc returns to bottom".into(),
            ),
            (
                "Esc · Ctrl+C".into(),
                "Stop work · close panel / clear draft".into(),
            ),
            ("Ctrl+Q".into(), "Cancel work and quit".into()),
            (
                "Alt+T".into(),
                "Inspect tools · arrows select · Enter details · Esc returns".into(),
            ),
            (
                "Ctrl+D in /resume or /drafts".into(),
                "Mark deletion · Enter confirms · Esc cancels".into(),
            ),
            (
                "Ctrl+S in /drafts".into(),
                "Send or queue a paused draft".into(),
            ),
        ]);
        self.local_finish(Kind::Notice, "Commands and controls", details);
    }

    pub(super) fn submit(&mut self) -> Result<bool, String> {
        if !self.imports.is_empty() {
            self.warn("Attachments are still importing. Wait before sending your draft.");
            return Ok(false);
        }
        if self.state.queue.is_editing() {
            if self.state.editor.prompt().is_empty() {
                self.warn("The draft is empty. Esc cancels; discard it in /drafts.");
            } else {
                self.finish_draft_edit(true);
            }
            return Ok(false);
        }
        let mut prompt = self.state.editor.prompt();
        if prompt.is_empty() {
            return Ok(false);
        }
        if (self.worker.is_none()
            || matches!(
                self.state.suggestions.chosen(),
                Some("/copy" | "/drafts" | "/attach")
            ))
            && self.state.suggestions.visible
            && prompt.attachments.is_empty()
            && let Some(chosen) = self.state.suggestions.chosen()
        {
            prompt = Prompt::plain(chosen);
        }
        let command = prompt.attachments.is_empty();
        if command
            && prompt
                .text
                .split_whitespace()
                .next()
                .is_some_and(|name| name.eq_ignore_ascii_case("/copy"))
        {
            self.state.history.submitted(&mut self.state.editor);
            self.edited();
            self.capture_input();
            self.copy_command(&prompt.text);
            return Ok(false);
        }
        if command && let Some(arguments) = attach_arguments(&prompt.text) {
            self.state.history.submitted(&mut self.state.editor);
            self.edited();
            self.capture_input();
            self.attach_command(arguments);
            return Ok(false);
        }
        if command && prompt.text.trim().eq_ignore_ascii_case("/drafts") {
            self.state.history.submitted(&mut self.state.editor);
            self.edited();
            self.open_drafts();
            self.capture_input();
            return Ok(false);
        }
        if self.worker.is_some() || self.deletion_job.is_some() {
            if let Err(error) = self.state.queue.push(prompt.clone()) {
                self.warn(&error);
                return Ok(false);
            }
            self.state.history.submitted(&mut self.state.editor);
            self.edited();
            return Ok(false);
        }
        self.state.history.submitted(&mut self.state.editor);
        self.edited();
        self.capture_input();
        self.dispatch(prompt)
    }

    pub(super) fn dispatch(&mut self, prompt: Prompt) -> Result<bool, String> {
        let command = prompt.attachments.is_empty() && prompt.text.trim_start().starts_with('/');
        if command && prompt.text.trim().eq_ignore_ascii_case("/drafts") {
            self.open_drafts();
            return Ok(false);
        }
        if command && self.copy_command(&prompt.text) {
            return Ok(false);
        }
        if command && let Some(arguments) = attach_arguments(&prompt.text) {
            self.attach_command(arguments);
            return Ok(false);
        }
        self.capture_input();
        self.state.clear_notice();
        self.state.information = None;
        if !command {
            if let Err(error) = self.agent.as_mut().unwrap().prepare_turn(&prompt) {
                self.state.message(Kind::Error, &error);
                self.keep_failed_prompt(prompt);
                return Ok(false);
            }
            self.state.history.record(&prompt);
            self.capture_input();
            self.state
                .message(Kind::User, &self.archive.redactor.text(&prompt.display()));
            self.state.activity = Some(Activity::new());
            self.worker = Some(Worker::start(self.agent.take().unwrap(), prompt));
            self.state.status = "Starting...".into();
            return Ok(false);
        }
        let value = prompt.text.trim();
        let (name, arguments) = value
            .split_once(char::is_whitespace)
            .map_or((value, ""), |(name, args)| (name, args.trim()));
        let name = name.to_ascii_lowercase();
        if matches!(name.as_str(), "/exit" | "/quit") && arguments.is_empty() {
            return Ok(true);
        }
        if name == "/help" && arguments.is_empty() {
            self.help();
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
        if matches!(name.as_str(), "/new" | "/clear") && arguments.is_empty() {
            self.cancel_copy();
            let result = self.agent.as_mut().unwrap().start_new(
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
        }
        self.local_start(value);
        match name.as_str() {
            "/tmp" => match crate::session::temporary::run(self.agent.as_mut().unwrap(), arguments)
            {
                Ok(report) => self.local_finish(Kind::Notice, &report.message, report.details),
                Err(error) => self.local_finish(Kind::Error, &error, vec![]),
            },
            "/new" | "/clear" if arguments.is_empty() => self.local_finish(
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
            "/settings" | "/setup" if arguments.is_empty() => {
                self.state.selector = Some(Selector::settings(
                    &self.config.settings.model,
                    self.config.settings.effort,
                ));
            }
            "/model" => {
                if !arguments.is_empty()
                    && let Err(error) = crate::openrouter::validate_model(arguments)
                {
                    self.local_finish(Kind::Error, &error, vec![]);
                    return Ok(false);
                }
                self.load_catalog(CatalogPurpose::Models {
                    defaults: false,
                    id: (!arguments.is_empty()).then(|| arguments.into()),
                });
            }
            "/effort" => match if arguments.is_empty() {
                Ok(None)
            } else {
                Effort::parse(arguments).map(Some)
            } {
                Ok(Some(Effort::Default)) => {
                    self.apply_selection(self.archive.model.clone(), Effort::Default, false)
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
        let information = self
            .pending_command
            .and_then(|index| self.state.items.get(index))
            .is_some_and(|item| {
                matches!(item, Item::Local { command, .. }
                if command.eq_ignore_ascii_case("/help") || command.eq_ignore_ascii_case("/tmp"))
            });
        if information && matches!(kind, Kind::Notice) && !details.is_empty() {
            if self
                .state
                .notice
                .as_ref()
                .is_none_or(|notice| !matches!(notice.kind, Kind::Error))
            {
                self.state.notice = None;
            }
            self.state.information = Some(super::information::Information::new(
                result.clone(),
                details.clone(),
            ));
        } else {
            self.state.notify(Feedback::result(kind, result.clone()));
        }
        self.local_finish_quiet(kind, &result, details);
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
            self.state.notice = Some(Feedback::result(Kind::Error, error.clone()));
            self.save_error = Some(error);
        }
    }

    pub(super) fn load_catalog(&mut self, purpose: CatalogPurpose) {
        self.state.selector = Some(Selector::loading());
        self.state.clear_notice();
        self.job = Some(Job::catalog(self.agent.as_ref().unwrap().api(), purpose));
    }
}

/// The path list of an `/attach` command.
fn attach_arguments(text: &str) -> Option<&str> {
    let text = text.trim_start();
    let (name, arguments) = text.split_once(char::is_whitespace).unwrap_or((text, ""));
    name.eq_ignore_ascii_case("/attach").then_some(arguments)
}
