//! Explicit workspace protection before a shell command. Only named files and
//! recursively named directories are observed; `.` names the workspace root.
use super::capture;
use super::inventory;
use super::policy::Policy;
use super::record::State;
use super::scan::in_scope;
use super::workspace::Workspace;
use crate::cancel::Cancellation;
use std::collections::BTreeSet;
use std::fs;
use std::sync::Arc;

#[cfg(test)]
mod tests;

// Inline observations hold at most 1 MiB each while a batch is recorded.
const BATCH_PATHS: usize = 16;

#[derive(Clone, Debug, Default)]
pub struct Selection {
    pub files: Vec<String>,
    /// Empty string is the workspace root. A trailing slash on an absent
    /// requested path declares an empty directory scope for future children.
    pub directories: Vec<String>,
}

#[derive(Clone, Debug, Default)]
pub struct Captured {
    pub group: Option<u64>,
    pub snapshot: Option<u64>,
    pub policy_id: u64,
    /// Journal frontier after all preflight observations and scope proofs.
    pub frontier: u64,
    pub selection: Selection,
    pub saved: usize,
    /// Each entry explains an explicitly requested path or incomplete subtree.
    pub partial: Vec<(String, String)>,
    pub complete_dirs: Vec<String>,
}

#[derive(Clone, Debug, Default)]
pub struct Reconciled {
    pub observed: usize,
    pub partial: Vec<(String, String)>,
}

fn cancelled(cancellation: &Cancellation) -> Result<(), String> {
    if cancellation.requested() {
        Err("Checkpoint capture was cancelled".into())
    } else {
        Ok(())
    }
}

fn within(root: &str, path: &str) -> bool {
    root.is_empty()
        || path
            .strip_prefix(root)
            .is_some_and(|tail| tail.is_empty() || tail.starts_with('/'))
}

fn normalize_request(path: &str) -> Result<(String, bool), String> {
    let path = path.replace('\\', "/");
    if path == "." || path == "./" {
        return Ok((String::new(), true));
    }
    let directory = path.ends_with('/');
    super::restore::normalize(&path).map(|path| (path, directory))
}

fn policy(workspace: &Workspace, cancellation: &Cancellation) -> Result<Policy, String> {
    let mut store = workspace.store_cancellable(cancellation)?;
    store.refresh_cancellable(cancellation)?;
    Ok(store.policy.clone())
}

fn select(
    workspace: &Workspace,
    paths: &[String],
    policy: &Policy,
    cancellation: &Cancellation,
    partial: &mut Vec<(String, String)>,
) -> Result<Selection, String> {
    let mut files = BTreeSet::new();
    let mut directories = BTreeSet::new();
    for requested in paths {
        cancelled(cancellation)?;
        let (path, directory_hint) = match normalize_request(requested) {
            Ok(path) => path,
            Err(reason) => {
                partial.push((requested.clone(), reason));
                continue;
            }
        };
        let path = if path.is_empty() {
            path
        } else {
            super::identity::relative(
                &workspace.root,
                &super::identity::absolute(&workspace.root, &path),
            )
            .unwrap_or(path)
        };
        // Classification needs only the entry type. Content is first read by
        // record_batch while the path lease is held.
        let metadata = (!path.is_empty()
            && super::files::confined_parent(&workspace.root, &path).is_ok())
        .then(|| fs::symlink_metadata(super::identity::absolute(&workspace.root, &path)).ok())
        .flatten();
        let directory = path.is_empty()
            || metadata.as_ref().is_some_and(inventory::ordinary_directory)
            || directory_hint;
        if !path.is_empty() && !in_scope(policy, &workspace.excluded, &path, directory) {
            partial.push((requested.clone(), "outside the checkpoint scope".into()));
        } else if directory {
            directories.insert(path);
        } else {
            files.insert(path);
        }
    }
    let mut roots: Vec<String> = directories.into_iter().collect();
    roots.sort_by_key(String::len);
    let mut minimal = Vec::new();
    for root in roots {
        if !minimal.iter().any(|parent: &String| within(parent, &root)) {
            minimal.push(root);
        }
    }
    files.retain(|file| !minimal.iter().any(|root| within(root, file)));
    Ok(Selection {
        files: files.into_iter().collect(),
        directories: minimal,
    })
}

fn listed(
    workspace: &Workspace,
    root: &str,
    policy: &Policy,
    cancellation: &Cancellation,
) -> Result<(Vec<String>, Option<String>), String> {
    cancelled(cancellation)?;
    if !root.is_empty() {
        if let Err(reason) = super::files::confined_parent(&workspace.root, root) {
            return Ok((vec![root.into()], Some(reason)));
        }
        match fs::symlink_metadata(super::identity::absolute(&workspace.root, root)) {
            Ok(metadata) if inventory::ordinary_directory(&metadata) => {}
            Ok(metadata) if metadata.is_file() => return Ok((vec![root.into()], None)),
            Ok(_) => return Ok((vec![root.into()], Some("not a directory".into()))),
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory
                ) =>
            {
                return Ok((vec![root.into()], None));
            }
            Err(error) => return Ok((vec![root.into()], Some(error.to_string()))),
        }
    }
    let walk = inventory::walk_cancellable(
        &workspace.root,
        root,
        policy,
        &workspace.excluded,
        cancellation,
    )?;
    let mut names: Vec<String> = walk.entries.into_iter().map(|entry| entry.path).collect();
    if !root.is_empty() {
        names.push(root.into());
    }
    names.sort();
    names.dedup();
    let issue = walk
        .limited
        .or_else(|| {
            (walk.unreadable > 0).then(|| format!("{} unreadable entries", walk.unreadable))
        })
        .or_else(|| {
            (!walk.unsupported.is_empty())
                .then(|| format!("{} unsupported names", walk.unsupported.len()))
        });
    Ok((names, issue))
}

fn include_recorded_absences(
    workspace: &Workspace,
    root: &str,
    policy: &Policy,
    names: &mut Vec<String>,
    cancellation: &Cancellation,
) -> Result<(), String> {
    let found: BTreeSet<String> = names.iter().cloned().collect();
    let store = workspace.store_cancellable(cancellation)?;
    for (path, versions) in &store.index.paths {
        cancelled(cancellation)?;
        if !within(root, path) || found.contains(path) {
            continue;
        }
        let directory = versions
            .last()
            .is_some_and(|version| matches!(version.state, State::Directory { .. }));
        if in_scope(policy, &workspace.excluded, path, directory) {
            names.push(path.clone());
        }
    }
    names.sort();
    Ok(())
}

fn record_batch(
    workspace: &Workspace,
    names: &[String],
    policy: &Policy,
    origin: &str,
    snapshot: Option<(u64, u64)>,
    cancellation: &Cancellation,
) -> Result<Vec<(String, State)>, String> {
    let leased: Vec<&str> = names.iter().map(String::as_str).collect();
    let _lease = workspace
        .leases
        .acquire_cancellable(&leased, cancellation)
        .map_err(|error| format!("Could not lease checkpoint paths: {error}"))?;
    let mut observed = Vec::with_capacity(names.len());
    for path in names {
        cancelled(cancellation)?;
        observed.push(capture::observe(
            &workspace.root,
            path,
            policy.max_file_bytes,
        ));
    }
    cancelled(cancellation)?;
    workspace
        .store_cancellable(cancellation)?
        .transaction_cancellable(false, cancellation, |transaction| {
            if transaction.store.policy.id() != policy.id() {
                return Err("checkpoint scope changed during capture; retry".into());
            }
            let mut saved = Vec::with_capacity(observed.len());
            for item in observed {
                cancelled(cancellation)?;
                let path = item.path.clone();
                let state = capture::record(transaction, item, origin, 0)?.map_or_else(
                    || transaction.index().current_state(&path),
                    |record| record.state,
                );
                if state.recoverable()
                    && let Some((group, before)) = snapshot
                {
                    super::recovery::add(transaction, group, before, &path, &state)?;
                }
                saved.push((path, state));
            }
            transaction.flush(false)?;
            Ok(saved)
        })
}

fn capture_names(
    workspace: &Workspace,
    names: &[String],
    policy: &Policy,
    origin: &str,
    snapshot: Option<(u64, u64)>,
    cancellation: &Cancellation,
    partial: &mut Vec<(String, String)>,
) -> Result<Vec<(String, State)>, String> {
    let mut saved = Vec::new();
    for batch in names.chunks(BATCH_PATHS) {
        cancelled(cancellation)?;
        match record_batch(workspace, batch, policy, origin, snapshot, cancellation) {
            Ok(records) => {
                for (path, state) in records {
                    if !state.recoverable() {
                        partial.push((
                            path.clone(),
                            format!("{} cannot be restored", state.label()),
                        ));
                    }
                    saved.push((path, state));
                }
            }
            Err(reason) if cancellation.requested() => return Err(reason),
            Err(reason) => {
                for path in batch {
                    partial.push((path.clone(), reason.clone()));
                }
            }
        }
    }
    Ok(saved)
}

fn verified(
    workspace: &Workspace,
    records: &[(String, State)],
    policy: &Policy,
    cancellation: &Cancellation,
) -> Result<bool, String> {
    for (path, state) in records {
        cancelled(cancellation)?;
        let live = capture::observe(&workspace.root, path, policy.max_file_bytes);
        let store = workspace.store_cancellable(cancellation)?;
        if !capture::matches_state_cancellable(&store, &live, state, true, cancellation)? {
            return Ok(false);
        }
    }
    Ok(true)
}

pub fn capture(
    workspace: &Arc<Workspace>,
    paths: &[String],
    origin: &str,
    cancellation: &Cancellation,
) -> Result<Captured, String> {
    cancelled(cancellation)?;
    if !paths.is_empty() {
        super::lifecycle::touch_throttled(&workspace.archive)?;
    }
    let policy = policy(workspace, cancellation)?;
    let mut result = Captured::default();
    result.policy_id = policy.id();
    result.selection = select(workspace, paths, &policy, cancellation, &mut result.partial)?;
    if result.selection.files.is_empty() && result.selection.directories.is_empty() {
        return Ok(result);
    }
    let (group, before) = workspace
        .store_cancellable(cancellation)?
        .transaction_cancellable(false, cancellation, |transaction| {
            let ids = super::recovery::begin_snapshot(transaction, origin, "before Bash")?;
            transaction.flush(false)?;
            Ok(ids)
        })?;
    result.group = Some(group);
    result.snapshot = Some(before);
    let files = result.selection.files.clone();
    let records = capture_names(
        workspace,
        &files,
        &policy,
        origin,
        Some((group, before)),
        cancellation,
        &mut result.partial,
    )?;
    result.saved += records
        .iter()
        .filter(|(_, state)| state.recoverable())
        .count();
    for (path, state) in records {
        if matches!(state, State::Directory { .. }) {
            result.partial.push((
                path.clone(),
                "became a directory during capture; descendants are not covered".into(),
            ));
        }
        if state.recoverable()
            && !verified(workspace, &[(path.clone(), state)], &policy, cancellation)?
        {
            result.partial.push((path, "changed during capture".into()));
            result.saved -= 1;
        }
    }
    let roots = result.selection.directories.clone();
    for root in roots {
        cancelled(cancellation)?;
        let (mut names, mut issue) = listed(workspace, &root, &policy, cancellation)?;
        if issue.is_none()
            && workspace
                .excluded
                .iter()
                .any(|excluded| within(&root, excluded))
        {
            issue = Some("contains Jecode-owned excluded paths".into());
        }
        let initially_present: BTreeSet<String> = names.iter().cloned().collect();
        include_recorded_absences(workspace, &root, &policy, &mut names, cancellation)?;
        let before_issues = result.partial.len();
        let records = capture_names(
            workspace,
            &names,
            &policy,
            origin,
            Some((group, before)),
            cancellation,
            &mut result.partial,
        )?;
        result.saved += records
            .iter()
            .filter(|(_, state)| state.recoverable())
            .count();
        if issue.is_none()
            && (records.len() != names.len() || result.partial.len() != before_issues)
        {
            issue = Some("some paths were not saved".into());
        }
        if issue.is_none() && !verified(workspace, &records, &policy, cancellation)? {
            issue = Some("paths changed during capture".into());
        }
        if issue.is_none() {
            let (now, now_issue) = listed(workspace, &root, &policy, cancellation)?;
            let current: BTreeSet<String> = now.into_iter().collect();
            if now_issue.is_some() || current != initially_present {
                issue = Some("directory membership changed during capture".into());
            }
        }
        if let Some(reason) = issue {
            result
                .partial
                .push((if root.is_empty() { ".".into() } else { root }, reason));
        } else {
            workspace
                .store_cancellable(cancellation)?
                .transaction_cancellable(false, cancellation, |transaction| {
                    if transaction.store.policy.id() != policy.id() {
                        return Err("checkpoint scope changed during capture; retry".into());
                    }
                    super::recovery::scope(transaction, group, before, &root)?;
                    transaction.flush(false)
                })?;
            result.complete_dirs.push(root);
        }
    }
    result.frontier = workspace
        .store_cancellable(cancellation)?
        .transaction_cancellable(true, cancellation, |transaction| {
            Ok(transaction.index().latest())
        })?;
    Ok(result)
}

/// Observe only declared command scopes, including newly created descendants
/// and deletions of previously known paths. This does not assign a writer.
pub fn reconcile(
    workspace: &Workspace,
    captured: &Captured,
    origin: &str,
    cancellation: &Cancellation,
) -> Result<Reconciled, String> {
    cancelled(cancellation)?;
    let policy = policy(workspace, cancellation)?;
    let mut result = Reconciled::default();
    if policy.id() != captured.policy_id {
        result
            .partial
            .push((".".into(), "checkpoint scope changed while Bash ran".into()));
    }
    let mut names: BTreeSet<String> = captured.selection.files.iter().cloned().collect();
    for root in &captured.selection.directories {
        let (mut listed, issue) = listed(workspace, root, &policy, cancellation)?;
        include_recorded_absences(workspace, root, &policy, &mut listed, cancellation)?;
        names.extend(listed);
        if let Some(reason) = issue {
            result.partial.push((root.clone(), reason));
        }
    }
    let names: Vec<String> = names.into_iter().collect();
    let records = capture_names(
        workspace,
        &names,
        &policy,
        origin,
        None,
        cancellation,
        &mut result.partial,
    )?;
    result.observed = records.len();
    workspace
        .store_cancellable(cancellation)?
        .transaction_cancellable(true, cancellation, |_| Ok(()))?;
    Ok(result)
}
