//! Parallel scope walk and batched capture. A scan is a sequence of path
//! observations, not an instantaneous multi-file image.
use super::capture;
use super::locks::Leases;
use super::policy::Policy;
use super::record::Record;
use super::store::Store;
use super::workspace::Workspace;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

pub(super) const WORKERS: usize = 8;
const BATCH_PATHS: usize = 1024;
const BATCH_STRIPES: usize = 8;

pub use super::inventory::walk;

#[derive(Clone, Copy)]
pub enum CaptureKind {
    Scan,
    Notification,
}

/// Paths Jecode owns that must never be captured, even inside a workspace.
pub fn physical_exclusions(root: &Path, owned: &[PathBuf]) -> Vec<String> {
    owned
        .iter()
        .filter_map(|path| fs::canonicalize(path).ok())
        .filter_map(|path| super::identity::relative(root, &path))
        .collect()
}

/// Whether a path and all of its ancestors are in the checkpoint scope.
pub fn in_scope(policy: &Policy, excluded: &[String], path: &str, directory: bool) -> bool {
    if excluded.iter().any(|owned| {
        if owned.is_empty() {
            return true;
        }
        #[cfg(windows)]
        let prefix = path
            .get(..owned.len())
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case(owned));
        #[cfg(not(windows))]
        let prefix = path.starts_with(owned);
        prefix && (path.len() == owned.len() || path.as_bytes().get(owned.len()) == Some(&b'/'))
    }) {
        return false;
    }
    path.bytes()
        .enumerate()
        .filter(|(_, byte)| *byte == b'/')
        .all(|(index, _)| policy.eligible(&path[..index], true))
        && policy.eligible(path, directory)
}

#[derive(Default, Debug)]
pub struct Summary {
    pub scanned: usize,
    pub candidates: usize,
    pub changed: Vec<(String, u64)>,
    pub excluded: usize,
    pub unsupported: usize,
    pub unreadable: usize,
    /// Only previously protected paths were examined; no absence may be
    /// inferred for paths that discovery did not reach.
    pub limited: Option<String>,
}

/// Captures every in-scope path whose metadata changed, plus deletions of
/// recorded paths. With `verify`, every file is reread and compared.
pub fn scan(
    workspace: &Workspace,
    origin: &str,
    verify: bool,
    cancellation: &crate::cancel::Cancellation,
) -> Result<Summary, String> {
    reconcile(workspace, origin, verify, false, cancellation)
}

/// Retries full discovery, including archives that used selective protection
/// under an earlier capacity policy.
pub fn scan_full(
    workspace: &Workspace,
    origin: &str,
    verify: bool,
    cancellation: &crate::cancel::Cancellation,
) -> Result<Summary, String> {
    reconcile(workspace, origin, verify, true, cancellation)
}

/// Explicit verification retries full discovery, so a workspace that was
/// previously selective can regain whole-scope protection.
pub fn discover(
    workspace: &Workspace,
    origin: &str,
    cancellation: &crate::cancel::Cancellation,
) -> Result<Summary, String> {
    reconcile(workspace, origin, true, true, cancellation)
}

fn reconcile(
    workspace: &Workspace,
    origin: &str,
    verify: bool,
    discover: bool,
    cancellation: &crate::cancel::Cancellation,
) -> Result<Summary, String> {
    let (root, policy) = {
        let mut store = workspace.store();
        store.refresh()?;
        workspace.observe_coverage(&store);
        store.counters.scans += 1;
        (store.workspace.root.clone(), store.policy.clone())
    };
    workspace.signals.update(|health| {
        if health.scope != policy.id() {
            health.scope = policy.id();
            health.selective = None;
            #[cfg(windows)]
            {
                health.policy = policy.clone();
            }
        }
    });
    if !discover && let Some(reason) = workspace.selective() {
        return tracked(workspace, &policy, origin, verify, reason, cancellation);
    }
    let excluded = &workspace.excluded;
    let walk = walk(&root, "", &policy, excluded);
    if let Some(reason) = walk.limited {
        workspace.limit_scope(&reason)?;
        return tracked(workspace, &policy, origin, verify, reason, cancellation);
    }
    workspace.signals.update(|health| health.selective = None);
    let mut candidates = Vec::new();
    {
        let store = workspace.store();
        let mut seen = HashSet::with_capacity(walk.entries.len());
        for entry in &walk.entries {
            seen.insert(entry.path.as_str());
            if (verify && !entry.directory)
                || !capture::fresh(
                    &store.index,
                    &entry.path,
                    &entry.stat,
                    entry.directory,
                    policy.max_file_bytes,
                )
            {
                candidates.push((entry.path.clone(), entry.stat.length));
            }
        }
        for (path, versions) in &store.index.paths {
            let Some(current) = versions.last() else {
                continue;
            };
            let directory = matches!(current.state, super::record::State::Directory { .. });
            if !seen.contains(path.as_str())
                && current.state.exists()
                && in_scope(&policy, excluded, path, directory)
                && !walk.unreadable_dirs.iter().any(|unreadable| {
                    unreadable.is_empty() || path.starts_with(&format!("{unreadable}/"))
                })
            {
                candidates.push((path.clone(), 0));
            }
        }
    }
    let mut summary = Summary {
        scanned: walk.entries.len(),
        candidates: candidates.len(),
        excluded: walk.excluded,
        unsupported: walk.unsupported.len(),
        unreadable: walk.unreadable,
        changed: Vec::new(),
        limited: None,
    };
    capture_paths(
        workspace,
        &policy,
        candidates,
        CaptureKind::Scan,
        origin,
        cancellation,
        &mut summary,
    )?;
    Ok(summary)
}

fn tracked(
    workspace: &Workspace,
    policy: &Policy,
    origin: &str,
    verify: bool,
    reason: String,
    cancellation: &crate::cancel::Cancellation,
) -> Result<Summary, String> {
    let recorded: Vec<_> = {
        let store = workspace.store();
        store
            .index
            .paths
            .iter()
            .map(|(path, versions)| {
                (
                    path.clone(),
                    versions
                        .last()
                        .map(|version| &version.state)
                        .is_some_and(|state| {
                            matches!(state, super::record::State::Directory { .. })
                        }),
                    store.index.stats.get(path).copied(),
                )
            })
            .collect()
    };
    let mut candidates = Vec::new();
    let mut summary = Summary {
        limited: Some(reason),
        ..Summary::default()
    };
    for (path, directory, previous) in recorded {
        if cancellation.requested() {
            return Err("Checkpoint capture was cancelled".into());
        }
        if !in_scope(policy, &workspace.excluded, &path, directory) {
            continue;
        }
        summary.scanned += 1;
        let current = fs::symlink_metadata(super::identity::absolute(&workspace.root, &path));
        let stat = current.as_ref().ok().map(super::record::Stat::of);
        if verify || stat != previous {
            candidates.push((path, stat.map_or(0, |stat| stat.length)));
        }
    }
    summary.candidates = candidates.len();
    capture_paths(
        workspace,
        policy,
        candidates,
        CaptureKind::Scan,
        origin,
        cancellation,
        &mut summary,
    )?;
    Ok(summary)
}

/// Observes paths with parallel reads and records them in batches,
/// each under the paths' leases so tool mutations are never interleaved.
/// Notification batches hold only a few lease stripes at a time, leaving
/// other paths available to concurrent tool calls.
pub fn capture_paths(
    workspace: &Workspace,
    policy: &Policy,
    mut candidates: Vec<(String, u64)>,
    kind: CaptureKind,
    origin: &str,
    cancellation: &crate::cancel::Cancellation,
    summary: &mut Summary,
) -> Result<(), String> {
    candidates.sort_by_cached_key(|(path, _)| Leases::stripe(path));
    let root = &workspace.root;
    let mut start = 0;
    while start < candidates.len() {
        if cancellation.requested() {
            return Err("Checkpoint capture was cancelled".into());
        }
        let end = batch_end(&candidates, start, policy.max_file_bytes, kind);
        let batch = &candidates[start..end];
        let names: Vec<&str> = batch.iter().map(|(path, _)| path.as_str()).collect();
        let _lease = workspace
            .leases
            .acquire(&names)
            .map_err(|error| format!("Could not coordinate checkpoint capture: {error}"))?;
        let captured = capture::observe_batches(root, &names, policy.max_file_bytes, |observed| {
            if cancellation.requested() {
                return Err("Checkpoint capture was cancelled".into());
            }
            let changed = workspace.store().transaction(false, |transaction| {
                let mut changed = Vec::new();
                for observed in observed {
                    let path = observed.path.clone();
                    if let Some(recorded) = capture::record(transaction, observed, origin, 0)? {
                        changed.push((path, recorded.seq));
                    }
                }
                Ok(changed)
            })?;
            summary.changed.extend(changed);
            Ok(())
        });
        match captured {
            Ok(()) => {}
            Err(error) if super::quota::is_full(&error) => {
                workspace.limit_scope(&error)?;
                summary.limited = Some(error);
                return Ok(());
            }
            Err(error) => return Err(error),
        }
        workspace.progress(end);
        start = end;
    }
    Ok(())
}

fn batch_end(
    candidates: &[(String, u64)],
    start: usize,
    max_file_bytes: u64,
    kind: CaptureKind,
) -> usize {
    let mut end = start;
    let mut bytes = 0;
    let mut stripes = 0;
    let mut previous = None;
    while end < candidates.len() && end - start < BATCH_PATHS && bytes < capture::BATCH_BYTES {
        if matches!(kind, CaptureKind::Notification) {
            let stripe = Leases::stripe(&candidates[end].0);
            if previous != Some(stripe) {
                if stripes == BATCH_STRIPES {
                    break;
                }
                stripes += 1;
                previous = Some(stripe);
            }
        }
        bytes = bytes.saturating_add(candidates[end].1.min(max_file_bytes));
        end += 1;
    }
    end
}

/// Records whether every in-scope path existing now has been recorded.
pub fn coverage(store: &mut Store, complete: bool, reason: &str) -> Result<(), String> {
    store.transaction(false, |transaction| {
        let previous = transaction.index().coverage.values().next_back();
        if transaction.index().coverage_complete() != complete
            || transaction.index().current_coverage().is_none()
            || (!complete
                && reason.starts_with("selective:")
                && previous.is_some_and(|(_, old)| old != reason))
        {
            let seq = transaction.seq();
            transaction.record(Record::Coverage {
                seq,
                complete,
                reason: reason.into(),
            });
        }
        Ok(())
    })
}

#[cfg(test)]
mod batch_tests {
    use super::*;

    #[test]
    fn stripe_batches_visit_each_path_in_order() {
        let mut stripes = HashSet::new();
        let mut candidates = Vec::new();
        for number in 0..1000 {
            let path = format!("p{number}");
            if stripes.insert(Leases::stripe(&path)) {
                candidates.push((path, 0));
                if candidates.len() == BATCH_STRIPES + 2 {
                    break;
                }
            }
        }
        candidates.sort_by_cached_key(|(path, _)| Leases::stripe(path));
        assert_eq!(candidates.len(), BATCH_STRIPES + 2);
        let first = batch_end(&candidates, 0, 10 << 20, CaptureKind::Notification);
        let second = batch_end(&candidates, first, 10 << 20, CaptureKind::Notification);
        assert_eq!(first, BATCH_STRIPES);
        assert_eq!(second, candidates.len());
        assert_eq!(
            batch_end(&candidates, 0, 10 << 20, CaptureKind::Scan),
            candidates.len()
        );
    }
}
