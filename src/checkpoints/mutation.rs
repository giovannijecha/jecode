//! Protection for changes made by tools. A file mutation durably records the
//! path's previous state and the intended content before the tool writes; a
//! command records a point before it runs and captures what it changed.
use super::capture;

use super::identity;
use super::locks::Lease;
use super::record::{Outcome, Record, State};
use super::scan::in_scope;
use super::workspace::{Readiness, Workspace};
use std::path::Path;
use std::sync::Arc;

mod preparation;

/// Whether a change can be reversed from the archive.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Protection {
    /// The previous state is saved at `before`; `after` is the recorded result.
    Protected { before: u64, after: Option<u64> },
    /// The path is outside the checkpoint scope.
    Excluded(String),
    /// The previous state could not be saved.
    Unprotected(String),
}

/// How completely a command's starting point reflects the workspace.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Freshness {
    /// Recorded paths were reconciled, subject to notification delivery lag.
    Current,
    /// The scope protects recorded paths and direct tool mutations.
    Selective(String),
    /// The first capture is running; uncaptured paths cannot be restored yet.
    Baseline(usize),
    /// Offline changes are being reconciled; the point may miss some of them.
    Reconciling(usize),
    /// The archive is unavailable; nothing before the command is saved.
    Unavailable(String),
}

pub struct Mutation<'a> {
    workspace: &'a Workspace,
    origin: String,
    kind: &'static str,
    path: Option<String>,
    lease: Option<Lease>,
    /// Prepare sequence, previous state and intended state.
    prepared: Option<(u64, State, State)>,
    protection: Protection,
}

impl Workspace {
    /// Starts protecting a change of one file. A path that cannot be
    /// protected still yields a mutation, so the tool proceeds regardless.
    pub fn mutation(&self, origin: &str, kind: &'static str, absolute: &Path) -> Mutation<'_> {
        let mut mutation = Mutation {
            workspace: self,
            origin: origin.into(),
            kind,
            path: None,
            lease: None,
            prepared: None,
            protection: Protection::Unprotected("the change was not prepared".into()),
        };
        let Some(path) = identity::relative(&self.root, absolute) else {
            mutation.protection = Protection::Excluded("outside the workspace".into());
            return mutation;
        };
        let policy = {
            let store = self.store();
            let _ = super::lifecycle::touch_throttled(&store.workspace.directory);
            store.policy.clone()
        };
        if !in_scope(&policy, &self.excluded, &path, false) {
            mutation.protection = Protection::Excluded("excluded by the checkpoint scope".into());
            return mutation;
        }
        if let Readiness::Failed(reason) = self.readiness() {
            mutation.protection = Protection::Unprotected(reason);
            return mutation;
        }
        match self.leases.acquire(&[&path]) {
            Ok(lease) => mutation.lease = Some(lease),
            Err(error) => {
                mutation.protection =
                    Protection::Unprotected(format!("could not coordinate the change: {error}"));
                return mutation;
            }
        }
        mutation.path = Some(path);
        mutation
    }
}

impl Mutation<'_> {
    /// Records the actual result and releases the path.
    pub fn finish(mut self, applied: bool) -> Protection {
        self.complete(applied);
        self.protection.clone()
    }

    fn complete(&mut self, applied: bool) {
        let Some((operation, before, after)) = self.prepared.take() else {
            self.lease = None;
            return;
        };
        let workspace = self.workspace;
        let path = self.path.clone().unwrap_or_default();
        let mut store = workspace.store();
        // The prepared object is the exact content supplied to the tool. A
        // metadata match lets us record that object without processing it a
        // second time; the next point still verifies the path from disk.
        let written = match &after {
            State::File { length, .. } if applied => {
                capture::written(&workspace.root, &path, *length)
            }
            _ => None,
        };
        let reread = written.is_some();
        let observed = written.unwrap_or_else(|| {
            capture::observe(&workspace.root, &path, store.policy.max_file_bytes)
        });
        let result = store.transaction(false, |transaction| {
            if reread {
                capture::record_prepared(transaction, observed, &after, &self.origin, operation)?;
            } else {
                capture::record(transaction, observed, &self.origin, operation)?;
            }
            transaction.flush(false)?;
            let index = transaction.index();
            let current = index.current(&path);
            let state = current.map_or(State::Absent, |version| version.state.clone());
            let outcome = if applied && same_content(&state, &after) {
                Outcome::Applied
            } else if !applied && state.same(&before) {
                Outcome::NotApplied
            } else {
                Outcome::Unknown
            };
            let after = current.map_or(index.latest(), |version| version.seq);
            let seq = transaction.seq();
            transaction.record(Record::Complete {
                seq,
                operation,
                outcome,
            });
            Ok(Some(after))
        });
        // The after-image may exceed the remaining archive budget even though
        // Prepare was durable. The reserved Complete frame closes that intent
        // as unknown, so recovery does not leave a pending operation behind.
        let mut capacity_error = None;
        let result = match result {
            Err(error) if super::quota::is_full(&error) => {
                capacity_error = Some(error);
                store.transaction(false, |transaction| {
                    let seq = transaction.seq();
                    transaction.record(Record::Complete {
                        seq,
                        operation,
                        outcome: Outcome::Unknown,
                    });
                    Ok(None)
                })
            }
            result => result,
        };
        drop(store);
        self.lease = None;
        if reread || capacity_error.is_some() {
            workspace.signals.update(|health| {
                health.dirty.insert(path.clone());
                if let Some(error) = capacity_error {
                    health.failure = Some(error);
                }
            });
        }
        match (result, &mut self.protection) {
            (Ok(seq), Protection::Protected { after, .. }) => *after = seq,
            (Err(error), Protection::Protected { .. }) => {
                // The previous state stays saved; only the result is unrecorded.
                workspace.signals.update(|health| {
                    health.dirty.insert(path.clone());
                    health.failure = Some(error);
                });
            }
            _ => {}
        }
    }
}

impl Drop for Mutation<'_> {
    fn drop(&mut self) {
        // A tool that failed after preparing may still have changed the path.
        self.complete(false);
    }
}

fn same_content(state: &State, intended: &State) -> bool {
    match (state, intended) {
        (
            State::File { length, digest, .. },
            State::File {
                length: expected,
                digest: wanted,
                ..
            },
        ) => length == expected && digest == wanted,
        (State::Oversized { length }, State::Oversized { length: expected }) => length == expected,
        _ => false,
    }
}

/// A shell command's protection: the point before it and what it changed.
pub struct Command {
    workspace: Arc<Workspace>,
    origin: String,
    pub point: Result<u64, String>,
    pub freshness: Freshness,
    finished: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CommandReport {
    pub before: Option<u64>,
    pub after: Option<u64>,
    pub changed: Vec<String>,
    pub error: Option<String>,
}

impl Workspace {
    /// Brings the archive up to date and records the point before a command.
    #[cfg(test)]
    pub fn command(self: &Arc<Self>, origin: &str, text: &str) -> Command {
        self.command_cancellable(origin, text, &crate::cancel::Cancellation::default())
    }

    pub fn command_cancellable(
        self: &Arc<Self>,
        origin: &str,
        text: &str,
        cancellation: &crate::cancel::Cancellation,
    ) -> Command {
        self.signals
            .update(|health| health.bash.push(origin.to_string()));
        let label: String = text.chars().take(120).collect();
        // A shell command can overwrite paths we cannot predict. Preserve
        // the settling window before it starts, as well as at completion;
        // ordinary turn points need no such fixed delay.
        let point = self.wait_ready_cancellable(cancellation).and_then(|()| {
            if !self.polling() {
                self.settle();
            }
            if cancellation.requested() {
                return Err("Operation cancelled".into());
            }
            self.mark("command", &label, origin, false, true)
        });
        let freshness = match self.readiness() {
            Readiness::Ready => self
                .selective()
                .map_or(Freshness::Current, Freshness::Selective),
            Readiness::Starting => Freshness::Baseline(0),
            Readiness::Baseline(count) => Freshness::Baseline(count),
            Readiness::Reconciling(count) => Freshness::Reconciling(count),
            Readiness::Failed(reason) => Freshness::Unavailable(reason),
        };
        let freshness = match &point {
            Err(error) => Freshness::Unavailable(error.clone()),
            Ok(_) => freshness,
        };
        Command {
            workspace: Arc::clone(self),
            origin: origin.into(),
            point,
            freshness,
            finished: false,
        }
    }
}

impl Command {
    /// Captures the command's changes and reports them.
    pub fn finish(mut self) -> CommandReport {
        let mut report = CommandReport {
            before: self.point.as_ref().ok().copied(),
            ..CommandReport::default()
        };
        // Filesystem observations do not identify the writer. Even during a
        // command, another process may have changed any reported path.
        if let Err(error) = self.workspace.catch_up_settled("external") {
            report.error = Some(error);
        }
        self.release();
        let Some(before) = report.before else {
            report.error = self.point.as_ref().err().cloned();
            return report;
        };
        let mut store = self.workspace.store();
        if let Err(error) = store.refresh() {
            report.error.get_or_insert(error);
        }
        let index = &store.index;
        report.after = Some(index.latest());
        let mut changed: Vec<String> = index
            .changed_between(before, index.latest())
            .into_iter()
            .map(|version| version.path.clone())
            .collect();
        changed.sort();
        changed.dedup();
        report.changed = changed;
        report
    }

    fn release(&mut self) {
        if std::mem::replace(&mut self.finished, true) {
            return;
        }
        let origin = &self.origin;
        self.workspace.signals.update(|health| {
            if let Some(position) = health.bash.iter().position(|running| running == origin) {
                health.bash.remove(position);
            }
        });
    }
}

impl Drop for Command {
    fn drop(&mut self) {
        self.release();
    }
}
