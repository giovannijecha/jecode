use super::super::{Issue, Snapshot, files, paths, state::State};
use crate::cancel::Cancellation;
use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

/// Return exact path stripes only when selection cannot expand into a tree.
/// The caller still inspects and validates the token after taking the lease.
pub(super) fn lease_paths(
    snapshot: &Snapshot,
    root: &Path,
    requested: Option<&[String]>,
) -> Option<Vec<String>> {
    let candidates: Vec<String> = match requested {
        Some(paths) if !paths.is_empty() => paths.to_vec(),
        Some(_) => return None,
        None if snapshot.roots.is_empty() && !snapshot.entries.is_empty() => {
            snapshot.entries.keys().cloned().collect()
        }
        None => return None,
    };
    let mut names = BTreeSet::new();
    for request in candidates {
        let normalized = paths::normalized_request(&request);
        if normalized.is_empty()
            || normalized == "."
            || request.ends_with('/')
            || request.ends_with('\\')
        {
            return None;
        }
        let saved = paths::resolve_saved(root, snapshot, &normalized).ok()?;
        let state = snapshot.entries.get(&saved)?;
        if matches!(state, State::Directory { .. } | State::Unreadable { .. })
            || snapshot.roots.contains(&saved)
            || snapshot
                .entries
                .keys()
                .any(|path| path != &saved && in_root(path, &saved))
            || snapshot.excluded.iter().any(|item| in_root(&saved, item))
        {
            return None;
        }
        files::confined_parent(root, &saved).ok()?;
        let absolute = paths::absolute(root, &saved);
        let live = paths::relative(root, &absolute)?;
        if live.to_lowercase() != saved.to_lowercase() {
            return None;
        }
        match fs::symlink_metadata(&absolute) {
            Ok(metadata) if metadata.is_dir() => return None,
            Ok(metadata) => {
                #[cfg(windows)]
                {
                    use std::os::windows::fs::MetadataExt;
                    if metadata.file_attributes() & 0x400 != 0 && !metadata.file_type().is_symlink()
                    {
                        return None;
                    }
                }
                #[cfg(not(windows))]
                let _ = metadata;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return None,
        }
        names.insert(saved);
    }
    Some(names.into_iter().collect())
}

pub(super) fn in_root(path: &str, root: &str) -> bool {
    root.is_empty()
        || path == root
        || path
            .strip_prefix(root)
            .is_some_and(|rest| rest.starts_with('/'))
}

/// A complete walk proves absence only within its original eligible scope.
/// A later explicit selection cannot widen that proof into excluded content.
pub(super) fn absence_proven(snapshot: &Snapshot, path: &str) -> bool {
    snapshot.roots.iter().any(|scope| {
        in_root(path, scope) && paths::eligible_beneath(path, scope, &snapshot.excluded)
    })
}

pub(super) fn selected(
    snapshot: &Snapshot,
    root: &Path,
    requested: Option<&[String]>,
    cancel: &Cancellation,
) -> Result<(Vec<String>, Vec<Issue>), String> {
    let mut names = BTreeSet::new();
    let mut issues = snapshot.partial.clone();
    let exact_undo = snapshot.tool.starts_with("undo ");
    let requests: Vec<String> = requested.map_or_else(
        || {
            snapshot
                .roots
                .iter()
                .cloned()
                .chain(
                    snapshot
                        .entries
                        .keys()
                        .filter(|path| !snapshot.roots.iter().any(|scope| in_root(path, scope)))
                        .cloned(),
                )
                .collect()
        },
        |paths| paths.to_vec(),
    );
    for request in requests {
        if cancel.requested() {
            return Err("Recovery inspection was cancelled".into());
        }
        let request = paths::normalized_request(&request);
        if std::path::Path::new(&request)
            .components()
            .any(|part| part == std::path::Component::ParentDir)
        {
            return Err(format!("Invalid workspace path: {request}"));
        }
        if exact_undo {
            match paths::resolve_saved(root, snapshot, &request) {
                Ok(saved) if !snapshot.excluded.iter().any(|item| in_root(&saved, item)) => {
                    names.insert(saved);
                }
                Ok(saved) => issues.push((saved, "Internal recovery storage is excluded".into())),
                Err(error) => issues.push((request, error)),
            }
            continue;
        }
        let relative = if request.is_empty() || request == "." {
            String::new()
        } else if let Ok(saved) = paths::resolve_saved(root, snapshot, &request) {
            saved
        } else if std::path::Path::new(&request).is_absolute() {
            paths::relative(root, std::path::Path::new(&request))
                .ok_or_else(|| format!("Path outside workspace: {request}"))?
        } else {
            if !request.split('/').all(files::normal_component) {
                return Err(format!("Invalid workspace path: {request}"));
            }
            paths::relative(root, &paths::absolute(root, &request))
                .ok_or_else(|| format!("Path outside workspace: {request}"))?
        };
        if !relative.is_empty()
            && snapshot
                .excluded
                .iter()
                .any(|item| in_root(&relative, item))
        {
            issues.push((relative, "Internal recovery storage is excluded".into()));
            continue;
        }
        let saved = snapshot.entries.contains_key(&relative) || absence_proven(snapshot, &relative);
        if !saved && !relative.is_empty() {
            issues.push((relative, "Not protected by this call".into()));
            continue;
        }
        let discovery = paths::discover(
            root,
            std::slice::from_ref(&relative),
            &snapshot.excluded,
            cancel,
        )?;
        issues.extend(discovery.partial);
        for path in discovery.paths {
            if let Some(saved) = paths::saved_alias(root, snapshot, &path) {
                names.insert(saved);
            } else if absence_proven(snapshot, &path) {
                names.insert(path);
            }
        }
        names.extend(
            snapshot
                .entries
                .keys()
                .filter(|path| in_root(path, &relative))
                .cloned(),
        );
        if !relative.is_empty() && saved {
            names.insert(relative);
        }
    }
    Ok((names.into_iter().collect(), issues))
}
