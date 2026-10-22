//! Protection for changes made by tools. A file mutation durably records the
//! path's previous state and the intended content before the tool writes; a
//! command acquires declared scopes and observes only those scopes afterward.
use super::capture;

use super::identity;
use super::locks::Lease;
use super::record::{Outcome, Record, State};
use super::scan::in_scope;
use super::workspace::Workspace;
use std::path::Path;

mod command;
mod preparation;
pub use command::{Command, Completion};

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
    cancellation: crate::cancel::Cancellation,
}

impl Workspace {
    /// Starts protecting a change of one file. A path that cannot be
    /// protected still yields a mutation, so the tool proceeds regardless.
    #[cfg(test)]
    pub fn mutation(&self, origin: &str, kind: &'static str, absolute: &Path) -> Mutation<'_> {
        self.mutation_cancellable(origin, kind, absolute, &self.cancellation)
    }

    pub fn mutation_cancellable(
        &self,
        origin: &str,
        kind: &'static str,
        absolute: &Path,
        cancellation: &crate::cancel::Cancellation,
    ) -> Mutation<'_> {
        let mut mutation = Mutation {
            workspace: self,
            origin: origin.into(),
            kind,
            path: None,
            lease: None,
            prepared: None,
            protection: Protection::Unprotected("the change was not prepared".into()),
            cancellation: cancellation.clone(),
        };
        let Some(path) = identity::relative(&self.root, absolute) else {
            mutation.protection = Protection::Excluded("outside the workspace".into());
            return mutation;
        };
        let policy = {
            let store = match self.store_cancellable(cancellation) {
                Ok(store) => store,
                Err(error) => {
                    mutation.protection = Protection::Unprotected(error);
                    return mutation;
                }
            };
            let _ = super::lifecycle::touch_throttled(&store.workspace.directory);
            store.policy.clone()
        };
        if !in_scope(&policy, &self.excluded, &path, false) {
            mutation.protection = Protection::Excluded("excluded by the checkpoint scope".into());
            return mutation;
        }
        match self.leases.acquire_cancellable(&[&path], cancellation) {
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
    pub fn check_cancelled(&self) -> Result<(), String> {
        if self.cancellation.requested() {
            Err("Operation cancelled".into())
        } else {
            Ok(())
        }
    }
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
        if self.cancellation.requested() {
            // Leave the durable intent for recovery rather than rereading a
            // large file or waiting behind a peer during interruption.
            self.lease = None;
            workspace.signals.update(|health| {
                health.interrupted.insert(operation);
                health.dirty.insert(path);
            });
            return;
        }
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
