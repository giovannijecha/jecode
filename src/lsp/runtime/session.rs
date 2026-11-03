//! Access to an instance's state. Releasing it publishes the status that
//! observers read, so observing an instance never waits for its I/O and never
//! mistakes the idle pump's maintenance for a request.
use super::{Client, Entry, State, Status};
use crate::cancel::Cancellation;
use std::ops::{Deref, DerefMut};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Mutex, MutexGuard, PoisonError, TryLockError};

/// Exclusive access to an instance's state.
pub(super) struct Session<'a> {
    entry: &'a Entry,
    state: MutexGuard<'a, State>,
}

/// A request counted as in progress from queueing until it returns.
pub(super) struct Pending<'a>(&'a Entry);

impl Entry {
    pub(super) fn new(id: String, root: PathBuf, root_source: String, fingerprint: String) -> Self {
        let observed = Status {
            server: id.clone(),
            root: Some(root.clone()),
            state: "not_started".into(),
            detail: root_source.clone(),
            capabilities: None,
            pid: None,
            generation: None,
        };
        Self {
            id,
            root,
            root_source,
            fingerprint,
            stop: Cancellation::default(),
            dirty: AtomicU64::new(0),
            requests: AtomicUsize::new(0),
            observed: Mutex::new(observed),
            state: Mutex::new(State {
                client: None,
                generation: 0,
                failures: 0,
                retry_at: None,
                status: "not_started".into(),
                detail: String::new(),
            }),
        }
    }

    pub(super) fn session(&self) -> Option<Session<'_>> {
        let state = self.state.lock().ok()?;
        Some(Session { entry: self, state })
    }

    pub(super) fn try_session(&self) -> Result<Session<'_>, TryLockError<()>> {
        match self.state.try_lock() {
            Ok(state) => Ok(Session { entry: self, state }),
            Err(TryLockError::WouldBlock) => Err(TryLockError::WouldBlock),
            Err(TryLockError::Poisoned(_)) => Err(TryLockError::Poisoned(PoisonError::new(()))),
        }
    }

    pub(super) fn pending(&self) -> Pending<'_> {
        self.requests.fetch_add(1, Ordering::AcqRel);
        Pending(self)
    }

    /// `busy` while a request is queued or running, else the last published status.
    pub(super) fn status(&self) -> Status {
        // Read before the snapshot: a finished request published before it stopped counting.
        if self.requests.load(Ordering::Acquire) > 0 {
            return Status {
                server: self.id.clone(),
                root: Some(self.root.clone()),
                state: "busy".into(),
                detail: "Request in progress".into(),
                capabilities: None,
                pid: None,
                generation: None,
            };
        }
        self.observed
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl Deref for Session<'_> {
    type Target = State;
    fn deref(&self) -> &State {
        &self.state
    }
}

impl DerefMut for Session<'_> {
    fn deref_mut(&mut self) -> &mut State {
        &mut self.state
    }
}

impl Drop for Session<'_> {
    /// Publishes while the state is still held, so snapshots follow lock order.
    fn drop(&mut self) {
        let state = &*self.state;
        let client = state.client.as_ref();
        let pid = client.map(Client::pid);
        let generation = client.map(|client| client.generation);
        let mut published = self
            .entry
            .observed
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        published.state.clone_from(&state.status);
        published.detail.clone_from(if state.detail.is_empty() {
            &self.entry.root_source
        } else {
            &state.detail
        });
        // Capabilities are fixed for a client's lifetime; idle ticks skip the copy.
        if published.pid != pid || published.generation != generation {
            published.capabilities = client.map(|client| client.capabilities.clone());
            published.pid = pid;
            published.generation = generation;
        }
    }
}

impl Drop for Pending<'_> {
    fn drop(&mut self) {
        self.0.requests.fetch_sub(1, Ordering::AcqRel);
    }
}
