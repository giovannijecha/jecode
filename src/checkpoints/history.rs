//! Operations on recorded history shared by the tool, the command line and
//! the interactive view: restores, pins and the archive policy.
use super::gc::{self, Collection};
use super::index::Index;
use super::index::reference;
use super::policy::{Policy, size};
use super::record::{Event, Record};
use super::restore::{self, Report, Request};
use super::scan;
use super::scopes::Selection;
use super::workspace::{Readiness, Workspace};
use std::collections::BTreeSet;
use std::time::Instant;

impl Workspace {
    /// Restores selected paths, or the whole scope, after bringing the
    /// archive up to date so conflicts compare with the actual state.
    pub fn restore(&self, request: &Request<'_>) -> Result<Report, String> {
        let readiness = self.readiness();
        if request.paths.is_none() && matches!(readiness, Readiness::Baseline(_)) {
            return Err(
                "The first capture of this workspace is still running; a full restore needs it to finish".into(),
            );
        }
        let captured = {
            let mut store = self.store();
            store.refresh()?;
            if restore::recovery_reference(&store.index, request.target) {
                Some(super::scopes::Captured {
                    selection: recovery_selection(&self.root, &store.index, request)?,
                    policy_id: store.policy.id(),
                    ..Default::default()
                })
            } else {
                None
            }
        };
        if let Some(captured) = captured {
            super::scopes::reconcile(self, &captured, "external", &self.cancellation)?;
        } else {
            self.catch_up("external")?;
        }
        let paths = restore::paths(&mut self.store(), request)?;
        let lease = match request.paths {
            None => self.leases.acquire_all(),
            Some(_) => {
                let names: Vec<&str> = paths.iter().map(String::as_str).collect();
                self.leases.acquire(&names)
            }
        };
        let _lease = lease.map_err(|error| format!("Could not coordinate the restore: {error}"))?;
        restore::restore(&mut self.store(), &paths, request)
    }

    /// Keeps a point regardless of the budget, or releases it again.
    pub fn pin(&self, target: u64, label: &str, pinned: bool) -> Result<(), String> {
        self.store().transaction(true, |transaction| {
            let index = transaction.index();
            if pinned && !index.retained(target) {
                return Err(format!(
                    "{} is no longer retained; the oldest retained point is {}",
                    reference(target),
                    reference(index.horizon)
                ));
            }
            if pinned == index.pins.contains_key(&target) {
                return Ok(());
            }
            let seq = transaction.seq();
            transaction.record(Record::Pin {
                seq,
                target,
                pinned,
                label: label.into(),
            });
            Ok(())
        })
    }

    /// Validates and saves new limits; every process applies them from the journal.
    pub fn configure(
        &self,
        budget_bytes: Option<u64>,
        max_file_bytes: Option<u64>,
        origin: &str,
    ) -> Result<Policy, String> {
        let mut store = self.store();
        let mut policy = store.policy.clone();
        if let Some(budget) = budget_bytes {
            policy.budget_bytes = budget;
        }
        if let Some(max) = max_file_bytes {
            policy.max_file_bytes = max;
        }
        policy.validate()?;
        if policy == store.policy {
            return Ok(policy);
        }
        let directory = store.workspace.directory.clone();
        store.transaction(true, |transaction| {
            policy.save_locked(&directory)?;
            let seq = transaction.seq();
            transaction.record(Record::Policy {
                seq,
                id: policy.id(),
                text: policy.encode(),
            });
            let seq = transaction.seq();
            transaction.record(Record::Event(Event {
                seq,
                origin: origin.into(),
                time: crate::checkpoints::clock::now(),
                text: format!(
                    "checkpoint limits changed: budget {}, maximum file {}",
                    size(policy.budget_bytes),
                    super::policy::file_limit(policy.max_file_bytes)
                ),
                paths: Vec::new(),
            }));
            Ok(())
        })?;
        store.policy = policy.clone();
        drop(store);
        // Files that crossed the size limit change state.
        self.signals.update(|health| {
            health.scope = policy.id();
            health.selective = Some("on-demand recovery; only captured paths are protected".into());
            health.collected = 0;
            health.collection_eligibility = None;
            #[cfg(windows)]
            {
                health.policy = policy.clone();
            }
            health.rescan = Some("the checkpoint limits changed".into());
        });
        Ok(policy)
    }

    /// Compacts the archive; without `force`, only when it exceeds its budget.
    pub fn collect(&self, force: bool) -> Result<Option<Collection>, String> {
        let (location, bytes, eligibility) = {
            let mut store = self.store();
            store.maintain()?;
            (
                store.workspace.clone(),
                store.journal.stored_bytes(),
                collection_eligibility(&store.index),
            )
        };
        let result = gc::collect(&location, force)
            .and_then(|collection| self.store().refresh().map(|()| collection));
        self.signals.update(|health| {
            // Record the input to the attempt, not the refreshed result. A
            // point or unpin added during compaction must trigger another try.
            health.collection_eligibility = Some(eligibility);
            health.collected = match &result {
                Ok(Some(collection)) => collection.after,
                _ => bytes,
            };
            match &result {
                Ok(Some(collection)) => {
                    health.collection = Some(Ok(collection.clone()));
                    health.collection_retry.clear();
                }
                Err(error) => {
                    health.collection = Some(Err(error.clone()));
                    health.collection_retry.failed(Instant::now());
                }
                Ok(None) => {
                    health.collection_retry.clear();
                    if health.collection.as_ref().is_some_and(Result::is_err) {
                        health.collection = None;
                    }
                }
            }
        });
        result
    }

    /// Rereads every file in scope to find edits that preserved size and
    /// modification time; returns the paths whose recorded state changed.
    pub fn verify(&self) -> Result<scan::Summary, String> {
        self.initialize()
    }

    /// Refreshes idle readers and collects off the hot path after enough
    /// growth or a change to the points and pins that constrain pruning.
    pub(super) fn maintain(&self) {
        let retry_blocked = self.signals.lock().collection_retry.blocked(Instant::now());
        let (bytes, budget, eligibility, expired) = {
            let mut store = self.store();
            if let Err(error) = store.maintain() {
                self.signals.update(|health| health.failure = Some(error));
                return;
            }
            (
                store.journal.stored_bytes(),
                store.policy.budget_bytes,
                collection_eligibility(&store.index),
                !retry_blocked && gc::due(&store.index, crate::checkpoints::clock::now()),
            )
        };
        let due = {
            let health = self.signals.lock();
            !retry_blocked
                && !health.collection_retry.blocked(Instant::now())
                && (expired
                    || health.readiness == Readiness::Ready
                        && (bytes > budget / 5 * 2
                            && (bytes >= health.collected.saturating_add(budget / 10)
                                || health.collection_eligibility.as_ref() != Some(&eligibility))))
        };
        if due {
            let _ = self.collect(false);
        }
    }
}

fn within(root: &str, path: &str) -> bool {
    root.is_empty()
        || path == root
        || path
            .strip_prefix(root)
            .is_some_and(|tail| tail.starts_with('/'))
}

fn recovery_selection(
    root: &std::path::Path,
    index: &Index,
    request: &Request<'_>,
) -> Result<Selection, String> {
    let saved = super::recovery::paths(index, request.target);
    let roots = super::recovery::roots(index, request.target);
    let Some(requested) = request.paths else {
        return Ok(Selection {
            files: saved,
            directories: roots,
        });
    };
    let mut files = BTreeSet::new();
    let mut directories = BTreeSet::new();
    let names = restore::TargetNames::new(root, index, request.target);
    for requested in requested {
        let requested = names.resolve(requested)?;
        // Even an entry absent from the saved scope needs a current-state
        // observation; recovery::target decides whether it can be restored.
        files.insert(requested.clone());
        if request.exact {
            continue;
        }
        files.extend(
            saved
                .iter()
                .filter(|path| within(&requested, path))
                .cloned(),
        );
        for root in &roots {
            if within(root, &requested) {
                directories.insert(requested.clone());
            } else if within(&requested, root) {
                directories.insert(root.clone());
            }
        }
    }
    let mut directories: Vec<String> = directories.into_iter().collect();
    directories.sort_by_key(String::len);
    let mut minimal = Vec::new();
    for directory in directories {
        if !minimal.iter().any(|root: &String| within(root, &directory)) {
            minimal.push(directory);
        }
    }
    Ok(Selection {
        files: files.into_iter().collect(),
        directories: minimal,
    })
}

fn collection_eligibility(index: &super::index::Index) -> (Option<u64>, Vec<u64>) {
    (
        index.points.keys().next_back().copied(),
        index.pins.keys().copied().collect(),
    )
}
