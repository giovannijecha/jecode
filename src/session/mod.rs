//! One ordered conversation owner with durable local history. Presentation never owns effects.
mod capabilities;
mod command;
mod context;
mod edit;
mod generation;
mod history;
pub mod persistence;
mod queue;
pub mod scope;
mod tool_loop;
mod types;
mod worker;

#[cfg(test)]
use crate::providers::openai_account::client;
use std::{
    io,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, SyncSender, TryRecvError},
    },
    thread::{self, JoinHandle},
};

pub(crate) use queue::Pending as PendingGuidance;
pub use types::*;
#[cfg(test)]
pub(crate) use worker::Backend as TestBackend;
pub(crate) const MAX_PROMPT_BYTES: usize = 8192;
pub(crate) const MAX_RECALLED_PROMPTS: usize = 64;

enum Command {
    Login,
    Logout,
    Prompt(String),
    Inspect,
    Compact,
    Model(Model),
    Catalog,
}
#[derive(PartialEq)]
enum Phase {
    Login,
    SignedOut,
    SigningOut,
    Ready,
    Generating,
    Updating,
    Closed,
}

pub struct Session {
    commands: Option<SyncSender<Command>>,
    events: Receiver<Event>,
    cancelled: Arc<AtomicBool>,
    stopped: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
    phase: Phase,
    turns: usize,
    guidance: Arc<queue::Pending>,
    queued: usize,
    catalog: Option<crate::providers::openai_account::catalog::Catalog>,
    selected: Model,
    initial_prompts: Vec<String>,
}
impl Session {
    pub fn start(model: Model) -> io::Result<Self> {
        Self::with_workspace(model, None)
    }
    pub fn with_workspace(
        model: Model,
        workspace: Option<crate::workspace::Workspace>,
    ) -> io::Result<Self> {
        let selected = workspace
            .as_ref()
            .map_or(std::path::Path::new("."), crate::workspace::Workspace::path)
            .to_owned();
        Self::with_directory(model, &selected, workspace)
    }
    pub fn with_directory(
        model: Model,
        directory: &std::path::Path,
        workspace: Option<crate::workspace::Workspace>,
    ) -> io::Result<Self> {
        let directory = scope::Directory::open(directory)?;
        if workspace
            .as_ref()
            .is_some_and(|w| !directory.contains(w.path()))
        {
            return Err(io::Error::other(
                "file-tool workspace differs from the selected directory",
            ));
        }
        let settings = crate::state::settings::Settings::user()?;
        let shell = crate::command::Shell::configured(
            settings.windows_powershell_executable.as_deref(),
            Some(directory.path()),
        )?;
        let history = persistence::create_in(
            &crate::state::Store::user()?,
            model,
            Some(directory.path()),
            workspace.as_ref(),
        )?;
        Self::with_history_shell(model, worker::Account::default(), workspace, history, shell)
    }
    pub fn resume(
        saved: persistence::Saved,
        directory: &scope::Directory,
        workspace: Option<crate::workspace::Workspace>,
    ) -> io::Result<Self> {
        directory.require(saved.directory.as_deref())?;
        if workspace.is_some() != saved.workspace.is_some()
            || workspace
                .as_ref()
                .is_some_and(|w| !directory.contains(w.path()))
            || workspace.as_ref().map_or(
                crate::workspace::Access::Workspace,
                crate::workspace::Workspace::access,
            ) != saved.access
        {
            return Err(io::Error::other(
                "saved workspace or file-access profile does not match the selected environment",
            ));
        }
        let settings = crate::state::settings::Settings::user()?;
        let shell = crate::command::Shell::configured(
            settings.windows_powershell_executable.as_deref(),
            Some(directory.path()),
        )?;
        Self::with_history_shell(
            saved.model,
            worker::Account::default(),
            workspace,
            saved.history,
            shell,
        )
    }
    #[cfg(test)]
    pub(crate) fn with_backend(
        model: Model,
        backend: impl worker::Backend + 'static,
        workspace: Option<crate::workspace::Workspace>,
    ) -> io::Result<Self> {
        Self::with_history(model, backend, workspace, history::History::default())
    }
    #[cfg(test)]
    fn with_history(
        model: Model,
        backend: impl worker::Backend + 'static,
        workspace: Option<crate::workspace::Workspace>,
        history: history::History,
    ) -> io::Result<Self> {
        Self::with_history_shell(
            model,
            backend,
            workspace,
            history,
            crate::command::Shell::default(),
        )
    }
    fn with_history_shell(
        model: Model,
        backend: impl worker::Backend + 'static,
        workspace: Option<crate::workspace::Workspace>,
        mut history: history::History,
        shell: crate::command::Shell,
    ) -> io::Result<Self> {
        #[cfg(test)]
        if history.record.is_none()
            && history.test_recovery.is_none()
            && let Some(workspace) = &workspace
        {
            let home = workspace.path().with_extension("home");
            std::fs::create_dir_all(&home)?;
            history.test_recovery = Some(crate::state::Store::in_home(&home)?);
        }
        let turns = history.turn_count();
        let initial_prompts = history
            .record
            .as_ref()
            .and_then(|record| record.recent_prompts())
            .unwrap_or_else(|| {
                history
                    .turns
                    .iter()
                    .rev()
                    .take(MAX_RECALLED_PROMPTS)
                    .map(|turn| turn.prompt.clone())
                    .collect::<Vec<_>>()
                    .into_iter()
                    .rev()
                    .collect()
            });
        history.environment = workspace.as_ref().map_or(String::new(), |workspace| {
            format!(
                "{} Command shell: {}. {}",
                workspace.instructions(),
                shell.label(),
                shell.limitation()
            )
        });
        history.shell = shell;
        // A pending prompt and a logout must both fit while the worker is cancelling.
        let (command_tx, command_rx) = mpsc::sync_channel(2);
        let (event_tx, event_rx) = mpsc::sync_channel(64);
        let guidance = Arc::new(queue::Pending::default());
        let cancelled = Arc::new(AtomicBool::new(false));
        let stopped = Arc::new(AtomicBool::new(false));
        let context = worker::Context {
            events: event_tx,
            cancelled: Arc::clone(&cancelled),
            stopped: Arc::clone(&stopped),
            guidance: Arc::clone(&guidance),
            next_effect: std::sync::atomic::AtomicU64::new(1),
            #[cfg(test)]
            effect_gate: history.effect_gate.clone(),
        };
        let worker = thread::Builder::new()
            .name("jecode-account".into())
            .spawn(move || worker::run(model, backend, command_rx, context, workspace, history))?;
        Ok(Self {
            commands: Some(command_tx),
            events: event_rx,
            cancelled,
            stopped,
            worker: Some(worker),
            phase: Phase::Login,
            turns,
            guidance,
            queued: 0,
            catalog: None,
            selected: model,
            initial_prompts,
        })
    }
    pub(crate) fn take_initial_prompts(&mut self) -> Vec<String> {
        std::mem::take(&mut self.initial_prompts)
    }
    /// False leaves ownership of the draft with the caller; nothing was queued.
    pub fn submit(&mut self, prompt: &str) -> bool {
        if self.phase != Phase::Ready
            || self.queued != 0
            || prompt.trim().is_empty()
            || prompt.len() > MAX_PROMPT_BYTES
            || self.selection_unavailable()
        {
            return false;
        }
        self.cancelled.store(false, Ordering::Release);
        if self
            .commands
            .as_ref()
            .is_none_or(|tx| tx.try_send(Command::Prompt(prompt.to_owned())).is_err())
        {
            return false;
        }
        self.turns += 1;
        self.phase = Phase::Generating;
        true
    }
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }
    pub fn login(&mut self) -> bool {
        if self.phase != Phase::SignedOut {
            return false;
        }
        self.cancelled.store(false, Ordering::Release);
        if self
            .commands
            .as_ref()
            .is_some_and(|tx| tx.try_send(Command::Login).is_ok())
        {
            self.phase = Phase::Login;
            true
        } else {
            false
        }
    }
    pub fn logout(&mut self) -> bool {
        if matches!(self.phase, Phase::SigningOut | Phase::Closed) {
            return false;
        }
        self.cancel();
        if self
            .commands
            .as_ref()
            .is_some_and(|tx| tx.try_send(Command::Logout).is_ok())
        {
            self.phase = Phase::SigningOut;
            true
        } else {
            false
        }
    }
    pub fn signed_out(&self) -> bool {
        self.phase == Phase::SignedOut
    }
    pub fn signing_out(&self) -> bool {
        self.phase == Phase::SigningOut
    }
    pub fn enqueue(&mut self, text: &str) -> bool {
        if self.phase != Phase::Generating
            || self.cancelled.load(Ordering::Acquire)
            || text.trim().is_empty()
            || text.len() > MAX_PROMPT_BYTES
            || self.queued >= 8
        {
            return false;
        }
        if !self.guidance.push(text) {
            return false;
        }
        self.queued += 1;
        true
    }
    /// A successful withdrawal transfers ownership back to the editor. A worker
    /// claim cannot be withdrawn, even if its UI event has not been polled yet.
    pub fn withdraw_latest(&mut self) -> Option<String> {
        let text = self.guidance.withdraw_latest()?;
        self.queued = self.queued.saturating_sub(1);
        Some(text)
    }
    pub(crate) fn pending_guidance(&self) -> Arc<queue::Pending> {
        Arc::clone(&self.guidance)
    }
    pub fn inspect_context(&mut self) -> bool {
        self.local_command(Command::Inspect)
    }
    pub fn ready(&self) -> bool {
        self.phase == Phase::Ready && self.queued == 0
    }
    pub fn catalog(&self) -> Option<&crate::providers::openai_account::catalog::Catalog> {
        self.catalog.as_ref()
    }
    pub fn selection_unavailable(&self) -> bool {
        self.catalog
            .as_ref()
            .filter(|catalog| catalog.fresh())
            .is_some_and(|catalog| {
                catalog.support(self.selected)
                    == crate::providers::openai_account::catalog::Support::Unsupported
            })
    }
    pub fn refresh_catalog(&mut self) -> bool {
        if self.ready() {
            self.cancelled.store(false, Ordering::Release);
            if self.local_command(Command::Catalog) {
                self.phase = Phase::Updating;
                return true;
            }
        }
        false
    }
    /// Model changes are ordered between turns and acknowledged after persistence.
    pub fn set_model(&mut self, model: Model) -> bool {
        if self.ready()
            && self
                .catalog
                .as_ref()
                .filter(|catalog| catalog.fresh())
                .is_none_or(|catalog| {
                    catalog.support(model)
                        != crate::providers::openai_account::catalog::Support::Unsupported
                })
            && self.local_command(Command::Model(model))
        {
            self.phase = Phase::Updating;
            true
        } else {
            false
        }
    }
    pub fn compact(&mut self) -> bool {
        if !self.ready() || self.selection_unavailable() {
            return false;
        }
        self.cancelled.store(false, Ordering::Release);
        if self.local_command(Command::Compact) {
            self.phase = Phase::Generating;
            true
        } else {
            false
        }
    }
    fn local_command(&self, command: Command) -> bool {
        self.phase == Phase::Ready
            && self
                .commands
                .as_ref()
                .is_some_and(|tx| tx.try_send(command).is_ok())
    }
    pub fn poll(&mut self) -> Option<Event> {
        match self.events.try_recv() {
            Ok(event) => {
                if matches!(event, Event::Guidance { .. } | Event::GuidanceReturned(_)) {
                    self.queued = self.queued.saturating_sub(1);
                }
                if matches!(event, Event::Guidance { new_turn: true, .. }) {
                    self.turns += 1;
                    self.phase = Phase::Generating;
                }
                match &event {
                    Event::CatalogLoaded(catalog) => self.catalog = Some(catalog.clone()),
                    Event::CatalogFailed(_) => self.catalog = None,
                    Event::ModelChanged(selected) => self.selected = *selected,
                    Event::LoggedOut => self.catalog = None,
                    _ => {}
                }
                self.phase = match &event {
                    Event::LoggedOut => Phase::SignedOut,
                    Event::LogoutFailed(_, true) => Phase::Ready,
                    Event::LogoutFailed(_, false) => Phase::SignedOut,
                    Event::Finished(End::Failed(Failure::Storage), _) => Phase::Closed,
                    Event::LoginFailed(Failure::Worker) => Phase::Closed,
                    _ if self.phase == Phase::SigningOut => return Some(event),
                    Event::Finished(end, _) if end.needs_login() => Phase::SignedOut,
                    Event::Ready | Event::ModelChanged(_) | Event::Finished(_, _) => Phase::Ready,
                    Event::LoginFailed(_) => Phase::SignedOut,
                    _ => return Some(event),
                };
                Some(event)
            }
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) if self.phase != Phase::Closed => {
                self.phase = Phase::Closed;
                Some(Event::LoginFailed(Failure::Worker))
            }
            Err(TryRecvError::Disconnected) => None,
        }
    }
}
impl Drop for Session {
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::Release);
        self.cancel();
        self.commands.take();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

#[cfg(test)]
mod account_tests;
#[cfg(all(test, any(windows, target_os = "linux")))]
mod capability_tests;
#[cfg(test)]
pub(crate) mod command_tests;
#[cfg(test)]
pub(crate) mod edit_tests;
#[cfg(all(test, any(windows, target_os = "linux")))]
pub(crate) mod outcome_backpressure_tests;
#[cfg(test)]
pub(crate) mod tests;
#[cfg(test)]
mod tool_tests;
