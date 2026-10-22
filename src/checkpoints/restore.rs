//! Reversible restores. The current state of every affected path is captured
//! durably before anything changes, and each restore is a new operation.
use super::capture::{self, Observation};
use super::files;
use super::index::{Index, reference};
use super::record::{Discovery, Event, Outcome, Prepare, Record, State, Version};
use super::scan::in_scope;
use super::store::Store;
use std::path::Path;

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

pub fn normalize(path: &str) -> Result<String, String> {
    let path = path.replace('\\', "/");
    let parts: Vec<&str> = path
        .split('/')
        .filter(|part| !part.is_empty() && *part != ".")
        .collect();
    if parts.is_empty()
        || path.starts_with('/')
        || path.get(1..2) == Some(":")
        || parts.contains(&"..")
        || parts.iter().any(|part| !files::normal_component(part))
    {
        return Err(format!(
            "Invalid restore path {path:?}; use a workspace-relative path"
        ));
    }
    Ok(parts.join("/"))
}

/// The paths a restore examines: selected paths with their recorded
/// descendants, or every recorded path for a full restore.
pub fn paths(store: &mut Store, request: &Request<'_>) -> Result<Vec<String>, String> {
    store.refresh()?;
    check_retained(&store.index, request.target)?;
    if let Expected::At(seq) = request.expected {
        check_retained(&store.index, seq)?;
    }
    match request.paths {
        Some(paths) => expand(&store.index, paths, request.target),
        None => Ok(store.index.paths.keys().cloned().collect()),
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
        capture::observe_batches(&root, paths, policy.max_file_bytes, |observed| {
            for observed in observed {
                let path = observed.path.clone();
                let present = !matches!(observed.kind, Observation::Absent);
                let directory = matches!(observed.kind, Observation::Directory(_));
                // A change found now is buffered, not yet in the index, and
                // differs from every expectation, so it conflicts.
                let recorded = capture::record(transaction, observed, &origin, 0)?;
                let index = transaction.index();
                let target_version = index.at(&path, target);
                let target_state = target_version.map_or(State::Absent, |v| v.state.clone());
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
                let identical = target_version.is_some_and(|version| version.seq == current_seq);
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
                    || !same_bytes(transaction.store, &expected_state, &current_state)
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
        report.before =
            Some(transaction.point("restore-before", &reference(target), &origin, false));
        let mut operations = Vec::new();
        for planned in &plan {
            let seq = transaction.seq();
            transaction.record(Record::Prepare(Prepare {
                seq,
                owner: owner.clone(),
                origin: origin.clone(),
                kind: kind.into(),
                path: planned.path.clone(),
                before: planned.current.clone(),
                after: planned.target.clone(),
                time: crate::sessions::now(),
            }));
            operations.push(seq);
        }
        Ok(operations)
    })?;
    let mut results = Vec::new();
    for (index, (planned, operation)) in plan.iter().zip(&operations).enumerate() {
        match install(store, &root, planned) {
            Ok(()) => {
                report.applied.push((planned.path.clone(), planned.action));
                results.push((*operation, planned, Outcome::Applied));
                #[cfg(test)]
                if index == 0 {
                    super::tests::process::stage("restore-first-installed");
                }
            }
            Err(error) => {
                report.failed = Some((planned.path.clone(), error));
                results.push((*operation, planned, Outcome::NotApplied));
                report.remaining = plan[index + 1..].iter().map(|p| p.path.clone()).collect();
                for (planned, operation) in plan[index + 1..].iter().zip(&operations[index + 1..]) {
                    results.push((*operation, planned, Outcome::NotApplied));
                }
                break;
            }
        }
    }
    let applied: Vec<String> = report
        .applied
        .iter()
        .map(|(path, _)| path.clone())
        .collect();
    let before = report.before.unwrap_or(target);
    let max = policy.max_file_bytes;
    report.after = Some(store.transaction(false, |transaction| {
        for (operation, planned, outcome) in &results {
            let mut outcome = *outcome;
            if outcome == Outcome::NotApplied {
                // Record what is actually there; the failed step may have left a change.
                let observed = capture::observe(&root, &planned.path, max);
                if let Some(recorded) = capture::record(transaction, observed, &origin, *operation)?
                {
                    outcome = if recorded.state.same(&planned.target) {
                        Outcome::Applied
                    } else {
                        Outcome::Unknown
                    };
                }
            } else {
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
                    time: crate::sessions::now(),
                    origin: origin.clone(),
                    operation: *operation,
                    discovery: Discovery::Known,
                }));
            }
            let seq = transaction.seq();
            transaction.record(Record::Complete {
                seq,
                operation: *operation,
                outcome,
            });
        }
        let after = transaction.point("restore", &reference(target), &origin, false);
        let seq = transaction.seq();
        transaction.record(Record::Event(Event {
            seq,
            origin: origin.clone(),
            time: crate::sessions::now(),
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

fn check_retained(index: &Index, seq: u64) -> Result<(), String> {
    if seq > index.latest() {
        return Err(format!("{} does not exist yet", reference(seq)));
    }
    if !index.retained(seq) {
        return Err(format!(
            "{} is no longer retained; the oldest retained checkpoint is {}",
            reference(seq),
            reference(index.horizon)
        ));
    }
    Ok(())
}

/// Complete coverage at `target` means every path existing then was already
/// recorded, so a path first recorded later did not exist at `target`.
fn created_after(index: &Index, path: &str, target: u64) -> bool {
    index
        .paths
        .get(path)
        .and_then(|versions| versions.first())
        .is_some_and(|first| first.seq > target && index.coverage_at(target))
}

fn expectation(index: &Index, path: &str, target: u64, request: &Request<'_>) -> (u64, State) {
    let at = |seq: u64| (seq, index.state_at(path, seq));
    match request.expected {
        Expected::At(seq) => at(seq),
        Expected::Latest => at(index.latest()),
        Expected::Own => index
            .paths
            .get(path)
            .and_then(|versions| {
                versions
                    .iter()
                    .rev()
                    .find(|version| version.seq > target && version.origin == request.origin)
            })
            .map_or_else(
                || at(target),
                |version| (version.seq, version.state.clone()),
            ),
    }
}

fn same_bytes(store: &Store, a: &State, b: &State) -> bool {
    match (a, b) {
        (State::File { object: x, .. }, State::File { object: y, .. }) if x != y => {
            match (store.bytes(*x), store.bytes(*y)) {
                (Ok(x), Ok(y)) => x[..] == y[..],
                _ => false,
            }
        }
        _ => true,
    }
}

/// Selected directories include their recorded descendants.
fn expand(index: &Index, paths: &[String], target: u64) -> Result<Vec<String>, String> {
    let mut expanded = std::collections::BTreeSet::new();
    for path in paths {
        let path = normalize(path)?;
        let prefix = format!("{path}/");
        for (candidate, versions) in index.paths.range(prefix.clone()..) {
            if !candidate.starts_with(&prefix) {
                break;
            }
            let visible = index
                .at(candidate, target)
                .is_some_and(|v| v.state.exists())
                || versions.last().is_some_and(|v| v.state.exists());
            if visible {
                expanded.insert(candidate.clone());
            }
        }
        expanded.insert(path);
    }
    Ok(expanded.into_iter().collect())
}

/// Delete descendants before replacing their parents; create directories
/// before restoring their descendants.
fn order(plan: &mut [Planned]) {
    plan.sort_by(|a, b| {
        let depth = |p: &Planned| p.path.matches('/').count();
        let rank = |p: &Planned| match (p.action, &p.target) {
            (Action::Delete, _) => 0,
            (_, State::Directory { .. }) => 1,
            _ => 2,
        };
        rank(a)
            .cmp(&rank(b))
            .then_with(|| match rank(a) {
                0 => depth(b).cmp(&depth(a)),
                _ => depth(a).cmp(&depth(b)),
            })
            .then_with(|| a.path.cmp(&b.path))
    });
}

fn install(store: &Store, root: &Path, planned: &Planned) -> Result<(), String> {
    files::confined_parent(root, &planned.path)?;
    let path = super::identity::absolute(root, &planned.path);
    let io = |error: std::io::Error| error.to_string();
    match &planned.target {
        State::Absent => match planned.current {
            State::Directory { .. } => std::fs::remove_dir(&path)
                .map_err(|error| format!("directory not removed: {error}")),
            _ => files::remove(&path).map_err(io),
        },
        State::File { object, mode, .. } => {
            let bytes = store.bytes(*object)?;
            if matches!(
                planned.current,
                State::Link { .. } | State::Directory { .. }
            ) {
                files::remove(&path).map_err(io)?;
            }
            files::install(&path, &bytes, *mode).map_err(io)
        }
        State::Directory { mode } => {
            if matches!(planned.current, State::Directory { .. }) {
                // Only the mode differs; the entries are restored separately.
                return mode.apply_path(&path).map_err(io);
            }
            if !matches!(planned.current, State::Absent) {
                files::remove(&path).map_err(io)?;
            }
            std::fs::create_dir(&path).map_err(io)?;
            if cfg!(unix) {
                mode.apply_path(&path).map_err(io)?;
            }
            Ok(())
        }
        State::Link { target } => {
            files::remove(&path).map_err(io)?;
            files::symlink(target, &path).map_err(io)
        }
        State::Oversized { .. } | State::Unreadable { .. } => {
            Err("the target state was not captured".into())
        }
    }
}
