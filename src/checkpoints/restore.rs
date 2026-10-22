//! Reversible restores. The current state of every affected path is captured
//! durably before anything changes, and each restore is a new operation.
use super::capture::{self, Observation};
use super::files;
use super::index::{Index, reference};
use super::record::{Discovery, Event, Outcome, Prepare, Record, State, Version};
use super::recovery;
use super::scan::in_scope;
use super::store::Store;

mod directories;
mod names;
mod selection;
pub(super) use names::TargetNames;
pub use selection::normalize;
use selection::{check_retained, created_after, expectation, same_bytes};
pub(super) use selection::{paths, recovery_reference, target_state};
mod installing;
use installing::{Failure, Step};

/// Which current state a restore may replace.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Expected {
    /// The requester's own latest version since the target, else the target.
    Own,
    /// The latest recorded version.
    Latest,
    /// The state at an explicit checkpoint.
    At(u64),
}

pub struct Request<'a> {
    pub target: u64,
    /// Selected paths; `None` restores the whole protected scope.
    pub paths: Option<&'a [String]>,
    /// Select only named entries, without expanding directory descendants.
    pub exact: bool,
    pub expected: Expected,
    pub preview: bool,
    pub origin: &'a str,
    pub excluded: &'a [String],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Replace,
    Create,
    Delete,
}

impl Action {
    pub fn label(self) -> &'static str {
        match self {
            Self::Replace => "replace",
            Self::Create => "create",
            Self::Delete => "delete",
        }
    }
}

pub struct Planned {
    pub path: String,
    pub action: Action,
    target: State,
    current: State,
    current_seq: u64,
}

pub struct Conflict {
    pub path: String,
    pub target: u64,
    pub expected: u64,
    pub current: u64,
}

#[derive(Default)]
pub struct Report {
    pub target: u64,
    pub preview: bool,
    /// Point holding the state saved before installation; restoring it undoes this restore.
    pub before: Option<u64>,
    pub after: Option<u64>,
    pub planned: Vec<(String, Action)>,
    pub applied: Vec<(String, Action)>,
    pub conflicts: Vec<Conflict>,
    pub skipped: Vec<(String, String)>,
    pub unchanged: usize,
    pub failed: Option<(String, String)>,
    pub remaining: Vec<String>,
}

impl Report {
    pub fn instructions_changed(&self) -> bool {
        self.applied
            .iter()
            .any(|(path, _)| path.rsplit('/').next() == Some("JECODE.md"))
    }
}

/// Restores `paths`, which the caller has leased.
pub fn restore(
    store: &mut Store,
    paths: &[String],
    request: &Request<'_>,
) -> Result<Report, String> {
    let target = request.target;
    let policy = store.policy.clone();
    let mut report = Report {
        target,
        preview: request.preview,
        ..Report::default()
    };
    let root = store.workspace.root.clone();
    let recorded_policy = store
        .index
        .policy_at(target)
        .cloned()
        .unwrap_or_else(|| policy.clone());
    for path in paths {
        files::confined_parent(&root, path)?;
    }
    // Observe and durably save the current state before deciding anything.
    let origin = request.origin.to_string();
    let durable = !request.preview;
    let mut plan = Vec::new();
    store.transaction(durable, |transaction| {
        // Retention may have advanced since the paths were chosen.
        check_retained(transaction.index(), target)?;
        if !request.preview {
            recovery::enable(transaction)?;
        }
        capture::observe_batches(&root, paths, policy.max_file_bytes, |observed| {
            for observed in observed {
                let path = observed.path.clone();
                let present = !matches!(observed.kind, Observation::Absent);
                let directory = matches!(observed.kind, Observation::Directory(_));
                // A change found now is buffered, not yet in the index, and
                // differs from every expectation, so it conflicts.
                let recorded = capture::record(transaction, observed, "external", 0)?;
                let index = transaction.index();
                let target_version = index.at(&path, target);
                let Some(target_state) = target_state(index, target, &path) else {
                    report
                        .skipped
                        .push((path, "not protected by this recovery point".into()));
                    continue;
                };
                let current = index.current(&path);
                let (current_state, current_seq) = match recorded {
                    Some(recorded) => (recorded.state, recorded.seq),
                    None => (
                        current.map_or(State::Absent, |v| v.state.clone()),
                        current.map_or(index.latest(), |v| v.seq),
                    ),
                };
                let target_directory = matches!(target_state, State::Directory { .. });
                if !in_scope(
                    &policy,
                    request.excluded,
                    &path,
                    directory || target_directory,
                ) || !in_scope(&recorded_policy, &[], &path, directory || target_directory)
                {
                    if request.paths.is_some() {
                        report
                            .skipped
                            .push((path, "outside the checkpoint scope".into()));
                    }
                    continue;
                }
                // Uncaptured states compare equal only as the same version.
                let identical = !recovery_reference(index, target)
                    && target_version.is_some_and(|version| version.seq == current_seq);
                if identical || target_state.recoverable() && target_state.same(&current_state) {
                    report.unchanged += 1;
                    continue;
                }
                if !target_state.recoverable() {
                    report.skipped.push((
                        path,
                        format!(
                            "{} at {}; its bytes were not captured",
                            target_state.label(),
                            reference(target)
                        ),
                    ));
                    continue;
                }
                if !current_state.recoverable() {
                    report.skipped.push((
                        path,
                        format!(
                            "current {} was not captured; restoring it could not be undone",
                            current_state.label()
                        ),
                    ));
                    continue;
                }
                if !target_state.exists()
                    && target_version.is_none()
                    && !recovery_reference(index, target)
                    && !created_after(index, &path, target)
                {
                    report.skipped.push((
                        path,
                        format!("not recorded at {}; it is kept", reference(target)),
                    ));
                    continue;
                }
                let (expected_seq, expected_state) = expectation(index, &path, target, request);
                if !expected_state.same(&current_state)
                    || !same_bytes(transaction.store, &expected_state, &current_state)?
                {
                    report.conflicts.push(Conflict {
                        path,
                        target,
                        expected: expected_seq,
                        current: current_seq,
                    });
                    continue;
                }
                let action = match (present, target_state.exists()) {
                    (true, true) => Action::Replace,
                    (false, _) => Action::Create,
                    (true, false) => Action::Delete,
                };
                report.planned.push((path.clone(), action));
                plan.push(Planned {
                    path,
                    action,
                    target: target_state,
                    current: current_state,
                    current_seq,
                });
            }
            // Save bounded live observations before reading another batch. The
            // final durable commit still precedes every workspace installation.
            transaction.flush(false)
        })?;
        Ok(())
    })?;
    // A full restore validates every path before changing any.
    if request.preview || plan.is_empty() || request.paths.is_none() && !report.conflicts.is_empty()
    {
        return Ok(report);
    }
    order(&mut plan);
    let owner = store.owner.clone();
    let kind = if request.paths.is_none() {
        "restore-all"
    } else {
        "restore"
    };
    let operations = store.transaction(true, |transaction| {
        let (group, snapshot) = recovery::begin_snapshot(
            transaction,
            &origin,
            &format!("restore {}", reference(target)),
        )?;
        transaction.flush(false)?;
        report.before = Some(snapshot);
        let mut operations = Vec::new();
        for planned in &plan {
            recovery::add(
                transaction,
                group,
                snapshot,
                &planned.path,
                &planned.current,
            )?;
            let seq = transaction.seq();
            transaction.record(Record::Prepare(Prepare {
                seq,
                owner: owner.clone(),
                origin: origin.clone(),
                kind: kind.into(),
                path: planned.path.clone(),
                before: planned.current.clone(),
                after: planned.target.clone(),
                time: crate::checkpoints::clock::now(),
            }));
            operations.push(seq);
        }
        Ok(operations)
    })?;
    let mut results = Vec::new();
    let mut directories = directories::Directories::default();
    let mut sync_failed = false;
    for (index, (planned, operation)) in plan.iter().zip(&operations).enumerate() {
        if let Err(failure) = directories.before(planned) {
            sync_failed = true;
            report.failed = Some(failure);
            report.remaining = plan[index..].iter().map(|p| p.path.clone()).collect();
            for (planned, operation) in plan[index..].iter().zip(&operations[index..]) {
                results.push((*operation, planned, Step::Untouched));
            }
            break;
        }
        match installing::install(store, &root, planned, policy.max_file_bytes) {
            Ok(()) => {
                directories.changed(&root, planned);
                report.applied.push((planned.path.clone(), planned.action));
                results.push((*operation, planned, Step::Applied));
                #[cfg(test)]
                if index == 0 {
                    super::tests::process::stage("restore-first-installed");
                }
            }
            Err(error) => {
                let step = match error {
                    Failure::Changed(observed) => {
                        let current = installing::save_conflict(store, *observed)?;
                        report.conflicts.push(Conflict {
                            path: planned.path.clone(),
                            target,
                            expected: planned.current_seq,
                            current,
                        });
                        Step::Untouched
                    }
                    Failure::Io(error) => {
                        report.failed = Some((planned.path.clone(), error));
                        Step::Failed
                    }
                };
                results.push((*operation, planned, step));
                report.remaining = plan[index + 1..].iter().map(|p| p.path.clone()).collect();
                for (planned, operation) in plan[index + 1..].iter().zip(&operations[index + 1..]) {
                    results.push((*operation, planned, Step::Untouched));
                }
                break;
            }
        }
    }
    if !sync_failed && let Err((path, error)) = directories.flush() {
        if let Some((_, previous)) = &mut report.failed {
            previous.push_str(&format!("; {error}"));
        } else {
            report.failed = Some((path, error));
        }
    }
    let before = report.before.unwrap_or(target);
    let max = policy.max_file_bytes;
    report.after = Some(store.transaction(false, |transaction| {
        for (operation, planned, step) in &results {
            let mut outcome = Outcome::NotApplied;
            if matches!(step, Step::Failed) {
                // Record what is actually there; the failed step may have left a change.
                let observed = capture::observe(&root, &planned.path, max);
                // A failed step cannot establish who wrote the observed state.
                let recorded = capture::record(transaction, observed, "external", 0)?;
                let actual = recorded.map_or_else(
                    || transaction.index().current_state(&planned.path),
                    |recorded| recorded.state,
                );
                outcome = if actual.same(&planned.current) {
                    Outcome::NotApplied
                } else {
                    Outcome::Unknown
                };
            } else if matches!(step, Step::Applied) {
                outcome = Outcome::Applied;
                let seq = transaction.seq();
                transaction.record(Record::Version(Version {
                    seq,
                    path: planned.path.clone(),
                    state: planned.target.clone(),
                    stat: std::fs::symlink_metadata(super::identity::absolute(
                        &root,
                        &planned.path,
                    ))
                    .ok()
                    .map(|metadata| super::record::Stat::of(&metadata)),
                    time: crate::checkpoints::clock::now(),
                    origin: origin.clone(),
                    operation: *operation,
                    discovery: Discovery::Known,
                }));
            }
            // Keep unsynced renames prepared. Recovery must reconsider them
            // after a crash, even if the live path currently has the target.
            if matches!(step, Step::Applied) && directories.pending(&root, planned) {
                continue;
            }
            let seq = transaction.seq();
            transaction.record(Record::Complete {
                seq,
                operation: *operation,
                outcome,
            });
        }
        let after = transaction.point("restore", &reference(target), &origin, false);
        let applied: Vec<String> = report
            .applied
            .iter()
            .map(|(path, _)| path.clone())
            .collect();
        let seq = transaction.seq();
        transaction.record(Record::Event(Event {
            seq,
            origin: origin.clone(),
            time: crate::checkpoints::clock::now(),
            text: format!(
                "{} {} path{} to {}; the previous state is saved as {}",
                if kind == "restore" {
                    "restored"
                } else {
                    "restored the workspace scope:"
                },
                applied.len(),
                if applied.len() == 1 { "" } else { "s" },
                reference(target),
                reference(before)
            ),
            paths: applied.iter().take(64).cloned().collect(),
        }));
        Ok(after)
    })?);
    Ok(report)
}

/// Delete descendants before replacing their parents; create directories
/// before restoring their descendants.
fn order(plan: &mut [Planned]) {
    plan.sort_by(|a, b| {
        let depth = |p: &Planned| p.path.matches('/').count();
        let rank = |p: &Planned| match (p.action, &p.target) {
            (Action::Delete, _) => 0,
            (_, State::Directory { mode })
                if matches!(p.current,
                    State::Directory { mode: current } if (mode.0 & 0o300) & !current.0 != 0
                ) =>
            {
                1
            }
            (_, State::Directory { .. }) if matches!(p.current, State::Directory { .. }) => 3,
            (_, State::Directory { .. }) => 1,
            _ => 2,
        };
        rank(a)
            .cmp(&rank(b))
            .then_with(|| match rank(a) {
                0 | 3 => depth(b).cmp(&depth(a)),
                _ => depth(a).cmp(&depth(b)),
            })
            .then_with(|| a.path.cmp(&b.path))
    });
}
