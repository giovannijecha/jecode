//! One ordered conversation owner with durable local history. Presentation never owns effects.
mod approval;
mod command;
mod context;
mod generation;
mod history;
pub mod persistence;
mod queue;
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

pub use types::*;

enum Command {
    Prompt(String),
    Inspect,
    Compact,
}
#[derive(PartialEq)]
enum Phase {
    Login,
    Ready,
    Generating,
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
    decisions: SyncSender<approval::Decision>,
    pending_approval: Option<u64>,
    guidance: SyncSender<String>,
    queued: usize,
}
impl Session {
    pub fn start(model: Model) -> io::Result<Self> {
        Self::with_workspace(model, None)
    }
    pub fn with_workspace(
        model: Model,
        workspace: Option<crate::workspace::Workspace>,
    ) -> io::Result<Self> {
        let history = persistence::create(
            &crate::state::Store::user()?,
            model,
            workspace.as_ref().map(crate::workspace::Workspace::path),
        )?;
        Self::with_history(model, worker::Account::default(), workspace, history)
    }
    pub fn resume(
        saved: persistence::Saved,
        workspace: Option<crate::workspace::Workspace>,
    ) -> io::Result<Self> {
        if workspace.as_ref().map(crate::workspace::Workspace::path) != saved.workspace.as_deref() {
            return Err(io::Error::other(
                "saved workspace does not match the selected directory",
            ));
        }
        Self::with_history(
            saved.model,
            worker::Account::default(),
            workspace,
            saved.history,
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
    fn with_history(
        model: Model,
        backend: impl worker::Backend + 'static,
        workspace: Option<crate::workspace::Workspace>,
        history: history::History,
    ) -> io::Result<Self> {
        let turns = history.turns.len();
        let (command_tx, command_rx) = mpsc::sync_channel(1);
        let (event_tx, event_rx) = mpsc::sync_channel(64);
        let (decision_tx, decision_rx) = mpsc::sync_channel(1);
        let (guidance_tx, guidance_rx) = mpsc::sync_channel(8);
        let cancelled = Arc::new(AtomicBool::new(false));
        let stopped = Arc::new(AtomicBool::new(false));
        let context = worker::Context {
            events: event_tx,
            cancelled: Arc::clone(&cancelled),
            stopped: Arc::clone(&stopped),
            decisions: decision_rx,
            guidance: guidance_rx,
            next_approval: std::sync::atomic::AtomicU64::new(1),
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
            decisions: decision_tx,
            pending_approval: None,
            guidance: guidance_tx,
            queued: 0,
        })
    }
    /// False leaves ownership of the draft with the caller; nothing was queued.
    pub fn submit(&mut self, prompt: &str) -> bool {
        if self.phase != Phase::Ready
            || self.queued != 0
            || self.turns >= history::MAX_TURNS
            || prompt.trim().is_empty()
            || prompt.len() > 8192
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
    pub fn enqueue(&mut self, text: &str) -> bool {
        if self.phase != Phase::Generating
            || self.cancelled.load(Ordering::Acquire)
            || text.trim().is_empty()
            || text.len() > 8192
            || self.queued >= 8
        {
            return false;
        }
        if self.guidance.try_send(text.into()).is_err() {
            return false;
        }
        self.queued += 1;
        true
    }
    pub fn inspect_context(&mut self) -> bool {
        self.local_command(Command::Inspect)
    }
    pub fn compact(&mut self) -> bool {
        if self.phase != Phase::Ready || self.queued != 0 {
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
    /// A decision applies to exactly one proposal received from this session.
    /// Stale, duplicated, unsolicited and cancelled decisions are rejected.
    pub fn decide(&mut self, id: u64, allow: bool) -> bool {
        if self.phase != Phase::Generating
            || self.pending_approval != Some(id)
            || self.cancelled.load(Ordering::Acquire)
        {
            return false;
        }
        if self
            .decisions
            .try_send(approval::Decision { id, allow })
            .is_err()
        {
            return false;
        }
        self.pending_approval = None;
        true
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
                    Event::EditProposed { id, .. } | Event::CommandProposed { id, .. } => {
                        self.pending_approval = Some(*id)
                    }
                    Event::EditFinished { .. }
                    | Event::CommandFinished { .. }
                    | Event::Finished(..)
                    | Event::LoginFailed(_) => self.pending_approval = None,
                    _ => {}
                }
                self.phase = match &event {
                    Event::Finished(End::Failed(Failure::Storage), _) => Phase::Closed,
                    Event::Ready | Event::Finished(_, _) => Phase::Ready,
                    Event::LoginFailed(_) => Phase::Closed,
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
pub(crate) mod command_tests;
#[cfg(test)]
pub(crate) mod edit_tests;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod tool_tests;
