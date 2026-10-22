mod activity;
mod attach;
mod cards;
#[cfg(test)]
mod checkpoint_tests;
mod checkpoints;
mod commands;
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
mod keys;
mod line;
mod markdown;
mod notice;
mod persistence;
mod poll;
mod processes;
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

use crate::{agent::Agent, export::Archive, openrouter::Model, session::SessionConfig};
use feedback::Feedback;
use jobs::Job;
use keys::{Decoded, Decoder};
use state::{Kind, State};
use std::sync::mpsc::RecvTimeoutError;
use std::time::Duration;
use std::time::Instant;
use terminal::{Input, Terminal};
use worker::Worker;

struct App {
    agent: Option<Agent>,
    worker: Option<Worker>,
    archive: Archive,
    config: SessionConfig,
    terminal: Option<Terminal>,
    state: State,
    renderer: display::Display,
    job: Option<Job>,
    catalog: Vec<Model>,
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
            config,
            renderer: display::Display::new((state.width, state.height)),
            terminal,
            state,
            job: None,
            catalog: vec![],
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
        }
    }
}

/// Opens the TUI; a non-empty `draft` starts the composer, as `--attach` does.
pub fn run(
    agent: Agent,
    config: SessionConfig,
    draft: crate::attachments::Prompt,
) -> Result<(), String> {
    run_inner(agent, config, None, draft)
}

pub fn resume(agent: Agent, config: SessionConfig, id: Option<String>) -> Result<(), String> {
    run_inner(agent, config, Some(id), Default::default())
}

fn run_inner(
    mut agent: Agent,
    config: SessionConfig,
    resume: Option<Option<String>>,
    draft: crate::attachments::Prompt,
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
    let mut decoder = Decoder::default();
    let mut dirty = true;
    let mut pending = None;
    let mut initial = app.terminal.as_mut().unwrap().take_initial();
    loop {
        app.terminal.as_mut().unwrap().check()?;
        dirty |= app.poll()?;
        if app.exit_requested {
            return Ok(());
        }
        // Consume complete geometry snapshots before painting. Bound this pass
        // so key bursts cannot starve worker events or the display.
        let events: Vec<_> = initial
            .drain(..)
            .chain(pending.take())
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
                dirty |= app.state.activity.is_some();
            }
        }
    }
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
mod tests;
#[cfg(test)]
mod tool_details_tests;
