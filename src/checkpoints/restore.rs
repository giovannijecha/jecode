//! Stateless inspection and guarded installation of one call's before image.
use super::{
    Issue, Snapshot, hash, paths,
    state::{self, State},
    store::Store,
};
use crate::cancel::Cancellation;
use std::collections::{BTreeMap, BTreeSet};

#[cfg(test)]
thread_local! {
    static AFTER_LEASE: std::cell::RefCell<Option<Box<dyn FnOnce()>>> = std::cell::RefCell::new(None);
    static BEFORE_UNDO_CAPTURE: std::cell::RefCell<Option<Box<dyn FnOnce()>>> = std::cell::RefCell::new(None);
    static AFTER_INSTALL: std::cell::RefCell<Option<Box<dyn FnOnce()>>> = std::cell::RefCell::new(None);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Create,
    Replace,
    Delete,
}
impl Action {
    pub fn label(self) -> &'static str {
        match self {
            Self::Create => "create",
            Self::Replace => "replace",
            Self::Delete => "delete",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Inspected {
    pub path: String,
    pub saved: State,
    pub current: State,
    pub action: Option<Action>,
}

#[derive(Clone, Debug)]
pub struct Inspection {
    pub reference: String,
    pub expected: String,
    pub selected: Vec<Inspected>,
    pub partial: Vec<Issue>,
}

#[derive(Clone, Debug)]
pub struct Undo {
    pub reference: String,
    pub paths: Vec<String>,
    pub expected: String,
}

#[derive(Clone, Debug)]
pub struct RestoreReport {
    pub reference: String,
    pub preview: bool,
    pub expected: String,
    pub planned: Vec<(String, Action)>,
    pub restored: Vec<(String, Action)>,
    pub conflicts: Vec<Issue>,
    pub failed: Vec<Issue>,
    pub skipped: Vec<Issue>,
    pub undo: Option<Undo>,
}

mod selecting;
use selecting::{absence_proven, lease_paths, selected};

fn token(selected: &[Inspected]) -> String {
    let mut bytes = Vec::new();
    for entry in selected {
        bytes.extend_from_slice(&(entry.path.len() as u64).to_le_bytes());
        bytes.extend_from_slice(entry.path.as_bytes());
        match &entry.current {
            State::Absent => bytes.push(0),
            State::File {
                length,
                digest,
                mode,
                ..
            } => {
                bytes.push(1);
                bytes.extend_from_slice(&length.to_le_bytes());
                bytes.extend_from_slice(&digest.0[0].to_le_bytes());
                bytes.extend_from_slice(&digest.0[1].to_le_bytes());
                bytes.extend_from_slice(&mode.0.to_le_bytes());
            }
            State::Directory { mode } => {
                bytes.push(2);
                bytes.extend_from_slice(&mode.0.to_le_bytes());
            }
            State::Link { target, directory } => {
                bytes.push(3);
                bytes.push(u8::from(*directory));
                bytes.extend_from_slice(&(target.len() as u64).to_le_bytes());
                bytes.extend_from_slice(target.as_bytes());
            }
            State::Unreadable { reason } => {
                bytes.push(4);
                bytes.extend_from_slice(&(reason.len() as u64).to_le_bytes());
                bytes.extend_from_slice(reason.as_bytes());
            }
        }
    }
    let digest = hash::digest(&bytes);
    format!("{:016x}{:016x}", digest.0[0], digest.0[1])
}

fn inspect_loaded(
    store: &Store,
    snapshot: &Snapshot,
    requested: Option<&[String]>,
    cancel: &Cancellation,
) -> Result<Inspection, String> {
    let (names, mut partial) = selected(snapshot, &store.root, requested, cancel)?;
    let mut selected = Vec::new();
    let mut observed = BTreeMap::<String, State>::new();
    for path in names {
        if cancel.requested() {
            return Err("Recovery inspection was cancelled".into());
        }
        let saved = snapshot
            .entries
            .get(&path)
            .cloned()
            .or_else(|| absence_proven(snapshot, &path).then_some(State::Absent));
        let Some(saved) = saved else {
            partial.push((path, "Not protected by this call".into()));
            continue;
        };
        let shadowed = path.match_indices('/').any(|(at, _)| {
            let ancestor = &path[..at];
            matches!(
                snapshot.entries.get(ancestor),
                Some(State::Directory { .. })
            ) && observed.get(ancestor).is_some_and(|state| {
                !matches!(state, State::Directory { .. } | State::Unreadable { .. })
            })
        });
        let current = if shadowed {
            State::Absent
        } else {
            match state::observe(&store.root, &path, cancel) {
                Ok(current) => current,
                Err(error) if cancel.requested() => return Err(error),
                Err(error) => {
                    partial.push((path.clone(), error.clone()));
                    State::Unreadable { reason: error }
                }
            }
        };
        let action = if !saved.recoverable() || !current.recoverable() || saved.same(&current) {
            None
        } else {
            Some(match (current.exists(), saved.exists()) {
                (false, true) => Action::Create,
                (true, false) => Action::Delete,
                _ => Action::Replace,
            })
        };
        observed.insert(path.clone(), current.clone());
        selected.push(Inspected {
            path,
            saved,
            current,
            action,
        });
    }
    let expected = token(&selected);
    Ok(Inspection {
        reference: snapshot.reference.clone(),
        expected,
        selected,
        partial,
    })
}

mod installing;
use installing::{finalize_directory, install, order, same_live, stage_file, stage_link};

impl Store {
    pub fn inspect(
        &self,
        reference: &str,
        requested: Option<&[String]>,
        cancel: &Cancellation,
    ) -> Result<Inspection, String> {
        let loaded = self.load(reference, cancel)?;
        inspect_loaded(self, &loaded.snapshot, requested, cancel)
    }

    pub fn restore(
        &self,
        reference: &str,
        requested: Option<&[String]>,
        expected: &str,
        preview: bool,
        cancel: &Cancellation,
    ) -> Result<RestoreReport, String> {
        let loaded = self.load(reference, cancel)?;
        let narrow = lease_paths(&loaded.snapshot, &self.root, requested);
        let mut lease = match &narrow {
            Some(names) => self.leases.acquire_cancellable(
                &names.iter().map(String::as_str).collect::<Vec<_>>(),
                cancel,
            ),
            None => self.leases.acquire_all_cancellable(cancel),
        }
        .map_err(|error| format!("Could not coordinate restore: {error}"))?;
        #[cfg(test)]
        AFTER_LEASE.with(|hook| {
            if let Some(action) = hook.borrow_mut().take() {
                action();
            }
        });
        let mut inspection = inspect_loaded(self, &loaded.snapshot, requested, cancel)?;
        if narrow.as_ref().is_some_and(|names| {
            inspection.selected.iter().any(|entry| {
                matches!(
                    entry.current,
                    State::Directory { .. } | State::Unreadable { .. }
                ) || names.binary_search(&entry.path).is_err()
            })
        }) {
            drop(lease);
            lease = self
                .leases
                .acquire_all_cancellable(cancel)
                .map_err(|error| format!("Could not coordinate restore: {error}"))?;
            inspection = inspect_loaded(self, &loaded.snapshot, requested, cancel)?;
        }
        let _lease = lease;
        let mut report = RestoreReport {
            reference: reference.into(),
            preview,
            expected: inspection.expected.clone(),
            planned: Vec::new(),
            restored: Vec::new(),
            conflicts: Vec::new(),
            failed: Vec::new(),
            skipped: inspection.partial.clone(),
            undo: None,
        };
        if expected != "latest" && expected != inspection.expected {
            report
                .conflicts
                .push(("".into(), "Current state changed since inspection".into()));
            return Ok(report);
        }
        let mut plan: Vec<_> = inspection
            .selected
            .into_iter()
            .filter_map(|entry| {
                if !entry.saved.recoverable() || !entry.current.recoverable() {
                    report.skipped.push((
                        entry.path,
                        "Saved or current state cannot be restored safely".into(),
                    ));
                    None
                } else {
                    entry.action.map(|_| entry)
                }
            })
            .collect();
        report.planned = plan
            .iter()
            .filter_map(|entry| entry.action.map(|action| (entry.path.clone(), action)))
            .collect();
        if preview || plan.is_empty() {
            return Ok(report);
        }
        // A damaged source must never cause a live deletion. Check all file payloads first.
        for entry in &plan {
            if matches!(entry.saved, State::File { .. }) {
                loaded.verify(&entry.saved, cancel)?;
            }
        }
        let names: Vec<String> = plan.iter().map(|entry| entry.path.clone()).collect();
        #[cfg(test)]
        BEFORE_UNDO_CAPTURE.with(|hook| {
            if let Some(action) = hook.borrow_mut().take() {
                action();
            }
        });
        let undo = self.capture_exact(&format!("undo {reference}"), "restore", &names, cancel)?;
        let Some(undo_reference) = undo.reference else {
            return Err("Could not save the current state before restore".into());
        };
        if !undo.partial.is_empty() {
            return Err("Undo capture was incomplete; restore was not started".into());
        }
        let undo_pack = self.load(&undo_reference, cancel)?;
        let mut undo_names = BTreeMap::new();
        let mut unique_undo = BTreeSet::new();
        for entry in &plan {
            let saved_name = paths::resolve_saved(&self.root, &undo_pack.snapshot, &entry.path)
                .map_err(|_| "Undo capture omitted a selected path; restore was not started")?;
            if !unique_undo.insert(saved_name.clone()) {
                return Err(
                    "Undo capture merged distinct selected paths; restore was not started".into(),
                );
            }
            let before = &undo_pack.snapshot.entries[&saved_name];
            if !before.same(&entry.current) {
                report.conflicts.push((
                    entry.path.clone(),
                    "Current state changed after inspection".into(),
                ));
            }
            undo_names.insert(entry.path.clone(), saved_name);
        }
        if !report.conflicts.is_empty() {
            return Ok(report);
        }
        order(&mut plan);
        let mut mutated = BTreeSet::new();
        let mut installed = BTreeMap::<String, State>::new();
        let mut directory_modes = Vec::new();
        for entry in &plan {
            if cancel.requested() {
                report
                    .failed
                    .push((entry.path.clone(), "Restore was cancelled".into()));
                break;
            }
            // Stage and verify every byte before an existing entry can be removed.
            let staged_file = match stage_file(&loaded, entry, &self.root, cancel) {
                Ok(staged) => staged,
                Err(error) => {
                    report.failed.push((entry.path.clone(), error));
                    break;
                }
            };
            let staged_link = match stage_link(entry, &self.root) {
                Ok(staged) => staged,
                Err(error) => {
                    report.failed.push((entry.path.clone(), error));
                    break;
                }
            };
            let before = &undo_pack.snapshot.entries[&undo_names[&entry.path]];
            let current = match state::observe(&self.root, &entry.path, cancel) {
                Ok(current) => current,
                Err(error) => {
                    report.failed.push((entry.path.clone(), error));
                    break;
                }
            };
            let path = paths::absolute(&self.root, &entry.path);
            match same_live(&undo_pack, before, &current, &path, cancel) {
                Ok(true) => {}
                Ok(false) => {
                    report.conflicts.push((
                        entry.path.clone(),
                        "Current bytes changed after the undo was saved".into(),
                    ));
                    break;
                }
                Err(error) => {
                    report.failed.push((entry.path.clone(), error));
                    break;
                }
            }
            match install(entry, &self.root, staged_file, staged_link) {
                Ok(()) => {
                    if let State::Directory { mode } = &entry.saved {
                        let created = !matches!(entry.current, State::Directory { .. });
                        directory_modes.push((entry.path.clone(), *mode, created));
                        if created {
                            mutated.insert(entry.path.clone());
                            installed.insert(
                                entry.path.clone(),
                                state::observe(&self.root, &entry.path, &Cancellation::default())
                                    .unwrap_or_else(|reason| State::Unreadable { reason }),
                            );
                        }
                    } else {
                        mutated.insert(entry.path.clone());
                        installed.insert(entry.path.clone(), entry.saved.clone());
                        if let Some(action) = entry.action {
                            report.restored.push((entry.path.clone(), action));
                        }
                    }
                    #[cfg(test)]
                    AFTER_INSTALL.with(|hook| {
                        if let Some(action) = hook.borrow_mut().take() {
                            action();
                        }
                    });
                }
                Err(error) => {
                    let after = state::observe(&self.root, &entry.path, &Cancellation::default());
                    let changed = match &after {
                        Ok(after) => {
                            !same_live(&undo_pack, before, after, &path, &Cancellation::default())
                                .unwrap_or(true)
                        }
                        Err(_) => true,
                    };
                    if changed {
                        mutated.insert(entry.path.clone());
                        installed.insert(
                            entry.path.clone(),
                            after.unwrap_or_else(|reason| State::Unreadable { reason }),
                        );
                    }
                    report.failed.push((entry.path.clone(), error));
                    break;
                }
            }
        }
        if report.failed.is_empty() && report.conflicts.is_empty() {
            directory_modes
                .sort_by_key(|(path, _, _)| std::cmp::Reverse(path.matches('/').count()));
            for (path, mode, created) in directory_modes {
                match finalize_directory(&self.root, &path, mode) {
                    Ok(()) => {
                        mutated.insert(path.clone());
                        installed.insert(path.clone(), State::Directory { mode });
                        report.restored.push((
                            path,
                            if created {
                                Action::Create
                            } else {
                                Action::Replace
                            },
                        ));
                    }
                    Err(error) => {
                        let before = &undo_pack.snapshot.entries[&undo_names[&path]];
                        let after = state::observe(&self.root, &path, &Cancellation::default());
                        let changed = after
                            .as_ref()
                            .map(|after| !before.same(after))
                            .unwrap_or(true);
                        if changed {
                            mutated.insert(path.clone());
                            installed.insert(
                                path.clone(),
                                after.unwrap_or_else(|reason| State::Unreadable { reason }),
                            );
                        }
                        report.failed.push((path, error));
                        break;
                    }
                }
            }
        } else if !directory_modes.is_empty() {
            report.failed.push((
                String::new(),
                "Directory modes were left accessible after partial restore".into(),
            ));
        }
        if !mutated.is_empty() {
            let mut undo_states = Vec::new();
            for path in &mutated {
                let name = undo_names[path].clone();
                let current = installed
                    .get(path)
                    .cloned()
                    .unwrap_or_else(|| State::Unreadable {
                        reason: "Installed state could not be verified".into(),
                    });
                undo_states.push(Inspected {
                    saved: undo_pack.snapshot.entries[&name].clone(),
                    path: name,
                    current,
                    action: None,
                });
            }
            undo_states.sort_by(|a, b| a.path.cmp(&b.path));
            let undo_paths = undo_states.iter().map(|entry| entry.path.clone()).collect();
            let expected = token(&undo_states);
            report.undo = Some(Undo {
                reference: undo_reference,
                paths: undo_paths,
                expected,
            });
        }
        Ok(report)
    }
}

#[cfg(test)]
#[path = "restore/tests.rs"]
mod tests;

#[cfg(test)]
mod scope_tests;
