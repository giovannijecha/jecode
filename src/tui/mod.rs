mod activity;
mod attach;
mod backend_command;
mod cards;
#[cfg(test)]
mod checkpoint_tests;
mod checkpoints;
mod commands;
mod conversations;
mod copy;
mod delete;
mod display;
mod draft_menu;
mod drafts;
mod editor;
mod feedback;
mod hint;
mod information;
mod input;
mod inspection;
mod jobs;
mod key_settings;
mod keys;
mod line;
mod markdown;
mod persistence;
mod poll;
mod processes;
mod prose;
mod providers;
mod recovery;
mod render;
mod resize;
mod routing;
mod selector;
mod settings;
mod state;
mod suggestions;
mod syntax;
#[cfg(test)]
mod temporary_tests;
mod terminal;
mod text;
mod theme;
mod transcript;
mod view;
mod viewport;
mod worker;

use crate::{agent::Agent, backend::Model, export::Archive, session::SessionConfig};
use feedback::Feedback;
use jobs::Job;
use keys::{Decoded, Decoder};
use state::{Kind, State};
use std::sync::mpsc::RecvTimeoutError;
use std::time::Duration;
use std::time::Instant;
use terminal::{Input, Terminal};
use worker::Worker;

pub(crate) struct StartupNotice {
    pub text: String,
    pub action: (&'static str, &'static str),
}

struct App {
    agent: Option<Agent>,
    worker: Option<Worker>,
    submission: Option<recovery::Submission>,
    archive: Archive,
    config: SessionConfig,
    terminal: Option<Terminal>,
    state: State,
    renderer: display::Display,
    job: Option<Job>,
    catalog: Vec<Model>,
    catalog_services: Option<crate::backend::Services>,
    provider_parent: Option<bool>,
    auth_accounts: Vec<crate::auth::openai::Account>,
    auth_account_index: Option<usize>,
    auth_updates: Option<std::sync::mpsc::Receiver<crate::auth::openai::SignInUpdate>>,
    auth_browser_url: Option<String>,
    auth_model_switch: bool,
    auth_signing_out: bool,
    pending_command: Option<usize>,
    exit_requested: bool,
    persistence: Option<crate::sessions::Handle>,
    saved_sessions: Vec<crate::sessions::Summary>,
    save_error: Option<String>,
    copy_targets: Vec<crate::copy::Target>,
    copy_job: Option<(String, crate::clipboard::Job)>,
    deletion_job: Option<delete::Job>,
    checkpoint_job: Option<checkpoints::Job>,
    imports: Vec<attach::Import>,
    settings_parent: Option<selector::Setting>,
    /// Reads the current conversation's processes, including while a worker owns the agent.
    process_observer: crate::tools::processes::Observer,
    #[cfg(test)]
    copied_text: Option<String>,
    #[cfg(test)]
    clipboard_image: Option<Result<Vec<u8>, String>>,
}

impl App {
    fn new(agent: Agent, config: SessionConfig, terminal: Option<Terminal>) -> Self {
        let state = State {
            width: terminal.as_ref().map_or(80, |terminal| terminal.size.0),
            height: terminal.as_ref().map_or(24, |terminal| terminal.size.1),
            effort: agent.effort().name().into(),
            queue: drafts::Queue::new(agent.inbox()),
            ..State::default()
        };
        let persistence = agent.sessions();
        let mut app = Self {
            archive: agent.archive(),
            process_observer: agent.processes(),
            agent: Some(agent),
            worker: None,
            submission: None,
            config,
            renderer: display::Display::new((state.width, state.height)),
            terminal,
            state,
            job: None,
            catalog: vec![],
            catalog_services: None,
            provider_parent: None,
            auth_accounts: vec![],
            auth_account_index: None,
            auth_updates: None,
            auth_browser_url: None,
            auth_model_switch: false,
            auth_signing_out: false,
            pending_command: None,
            exit_requested: false,
            persistence,
            saved_sessions: vec![],
            save_error: None,
            copy_targets: vec![],
            copy_job: None,
            deletion_job: None,
            checkpoint_job: None,
            imports: Vec::new(),
            settings_parent: None,
            #[cfg(test)]
            copied_text: None,
            #[cfg(test)]
            clipboard_image: None,
        };
        app.session_hint();
        app.refresh_context();
        app
    }

    /// Reads the engine's projection while the application holds the agent;
    /// a running turn reports its own through events.
    fn refresh_context(&mut self) {
        if let Some(agent) = &self.agent {
            self.state.context = Some(agent.context_status());
            self.state.chatgpt_account = if agent.backend_id() == "openai" {
                let identity = agent.services().account();
                let account = crate::auth::openai::Store::new(
                    self.config
                        .store
                        .path()
                        .parent()
                        .expect("configuration directory")
                        .to_path_buf(),
                )
                .accounts()
                .ok()
                .and_then(|accounts| {
                    accounts.into_iter().find(|account| {
                        identity.as_deref()
                            == Some(format!("{}:{}", account.client_id, account.subject).as_str())
                    })
                });
                Some(match account {
                    Some(account) if account.enabled => {
                        state::ChatGptAccount::Active(account.email)
                    }
                    Some(_) => state::ChatGptAccount::SignedOut,
                    None => state::ChatGptAccount::Unavailable,
                })
            } else {
                None
            };
        }
    }

    /// A turn, deletion or restore is running; new prompts wait in the queue.
    fn busy(&self) -> bool {
        self.worker.is_some() || self.deletion_job.is_some() || self.checkpoint_job.is_some()
    }

    /// Nothing runs in the background, including other jobs.
    fn idle(&self) -> bool {
        !self.busy() && self.job.is_none()
    }
}

/// Opens the TUI; a non-empty `draft` starts the composer, as `--attach` does.
pub(crate) fn launch(
    mut agent: Agent,
    config: SessionConfig,
    resume: Option<Option<String>>,
    draft: crate::attachments::Prompt,
    notice: Option<StartupNotice>,
) -> Result<(), String> {
    agent.enable_sessions(
        config
            .store
            .path()
            .parent()
            .expect("configuration directory"),
    )?;
    let terminal = Terminal::open(&config.bash)?;
    let mut app = App::new(agent, config, Some(terminal));
    if !draft.is_empty() {
        app.state.editor.set(draft);
    }
    match resume {
        Some(Some(id)) => app.resume_session(&id),
        Some(None) => app.open_sessions(),
        None => {}
    }
    app.restore_staged();
    if let Some(notice) = notice {
        let mut feedback = Feedback::result(Kind::Warning, app.archive.redactor.text(&notice.text));
        feedback.action = Some(notice.action);
        app.state.notify(feedback);
    }
    let mut decoder = Decoder::default();
    let mut dirty = true;
    let mut pending = None;
    loop {
        app.terminal.as_mut().unwrap().check()?;
        dirty |= app.poll()?;
        if app.exit_requested {
            return Ok(());
        }
        // Consume complete geometry snapshots before painting. Bound this pass
        // so key bursts cannot starve worker events or the display.
        let events: Vec<_> = pending
            .take()
            .into_iter()
            .chain(app.terminal.as_ref().unwrap().input.try_iter().take(64))
            .collect();
        for event in events {
            if app.terminal_event(event, &mut decoder)? {
                return Ok(());
            }
            dirty = true;
        }
        if dirty {
            app.capture_input();
        }
        dirty |= app.persist_input(false);
        if dirty || app.renderer.needs_draw(&app.state, Instant::now()) {
            app.renderer.draw(
                &app.state,
                &app.archive.redactor.text(&app.archive.model),
                &app.archive
                    .redactor
                    .text(&app.archive.directory.to_string_lossy()),
            )?;
            dirty = false;
        }
        match app.terminal.as_ref().unwrap().input.recv_timeout({
            let now = Instant::now();
            let delay = app
                .state
                .feedback_wait(now, app.renderer.wait(&app.state, now));
            let delay = if app.worker.is_some() || app.checkpoint_job.is_some() {
                delay.min(Duration::from_millis(16))
            } else {
                delay
            };
            if app.copy_job.is_some() || !app.imports.is_empty() {
                delay.min(Duration::from_millis(20))
            } else if loading(&app.state) {
                // The scanner advances every 120 ms.
                delay.min(Duration::from_millis(120))
            } else if app.state.processes.is_some() {
                delay.min(processes::REFRESH)
            } else {
                delay
            }
        }) {
            Ok(event) => pending = Some(event),
            Err(RecvTimeoutError::Disconnected) => {
                return Err("Terminal input closed. Use jecode --plain.".into());
            }
            Err(RecvTimeoutError::Timeout) => {
                if let Some(decoded) = decoder.idle() {
                    if app.input(decoded)? {
                        return Ok(());
                    }
                    dirty = true;
                }
                dirty |= app.state.activity.is_some() || loading(&app.state);
            }
        }
    }
}

/// A scanner runs outside a turn: a waiting panel or idle progress.
fn loading(state: &state::State) -> bool {
    state.current_progress().is_some()
        || state
            .selector
            .as_ref()
            .is_some_and(|menu| matches!(menu.purpose, selector::Purpose::Loading))
}

impl App {
    fn terminal_event(&mut self, event: Input, decoder: &mut Decoder) -> Result<bool, String> {
        match event {
            #[cfg(unix)]
            Input::Bytes(bytes) => {
                for decoded in decoder.bytes(&bytes) {
                    if self.input(decoded)? {
                        return Ok(true);
                    }
                }
            }
            #[cfg(any(windows, test))]
            Input::Key(key) => {
                for decoded in decoder.key(key) {
                    if self.input(decoded)? {
                        return Ok(true);
                    }
                }
            }
            #[cfg(any(windows, test))]
            Input::Scroll(amount) => {
                self.input(Decoded::Scroll(amount))?;
            }
            #[cfg(windows)]
            Input::Modes(..) => {}
            Input::Size(size) => {
                self.state.width = size.width;
                self.state.height = size.height;
                self.renderer
                    .resized((size.width, size.height), Instant::now());
            }
            #[cfg(any(windows, test))]
            Input::PasteOverflow => {
                *decoder = Decoder::default();
                self.input(Decoded::Error)?;
            }
            #[cfg(any(windows, test))]
            Input::Paste(text) => match decoder.paste(&text) {
                Ok(text) if !text.is_empty() => {
                    self.input(Decoded::Paste(text))?;
                }
                Err(()) => {
                    self.input(Decoded::Error)?;
                }
                _ => {}
            },
            Input::Error(error) => return Err(error),
        }
        Ok(false)
    }
}

impl Drop for App {
    fn drop(&mut self) {
        if let Some(worker) = &self.worker {
            worker.cancel();
        }
        // Finish process cleanup before returning control to the caller's shell.
        drop(self.copy_job.take());
        self.imports.clear();
        drop(self.worker.take());
        // The worker may have accepted input whose event was never polled.
        self.reconcile_accepted();
        drop(self.job.take());
        self.finish_checkpoints(true);
        let deletion_error = self.finish_delete(true).err();
        self.persist_input(true);
        self.collect_attachments();
        let fullscreen = self.terminal.is_some();
        // The guardian leaves the alternate screen before any shell output.
        drop(self.terminal.take());
        if fullscreen {
            self.print_exit();
        }
        if let Some(error) = deletion_error {
            eprintln!("{}", self.archive.redactor.text(&error));
        }
        if let Some(error) = &self.save_error {
            eprintln!("{error}");
        }
    }
}

mod exit;

#[cfg(test)]
mod auto_compact_tests;
#[cfg(test)]
mod conversation_tests;
#[cfg(test)]
mod copy_tests;
#[cfg(test)]
mod delete_tests;
#[cfg(test)]
mod feedback_tests;
#[cfg(test)]
mod interaction_tests;
#[cfg(test)]
mod intraturn_tests;
#[cfg(test)]
mod long_work_smoke;
#[cfg(test)]
mod manual_tests;
#[cfg(test)]
mod measurement_tests;
#[cfg(test)]
mod persistence_smoke;
#[cfg(test)]
mod persistence_tests;
#[cfg(test)]
mod routing_tests;
#[cfg(test)]
mod session_menu_tests;
#[cfg(test)]
mod settings_tests;
#[cfg(test)]
mod shortcut_tests;
#[cfg(test)]
mod spacing_tests;
#[cfg(test)]
mod specimen;
#[cfg(test)]
mod startup_session_tests;
#[cfg(test)]
mod startup_tests;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod tool_details_tests;
