//! Path selection and per-path recovery preconditions.
use super::*;

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
    if let Expected::At(seq) = request.expected
        && (seq > store.index.latest() || seq < store.index.horizon && !store.index.retained(seq))
    {
        return Err(format!(
            "{} is not available as a restore expectation",
            reference(seq)
        ));
    }
    let names = super::TargetNames::new(&store.workspace.root, &store.index, request.target);
    match request.paths {
        Some(paths) if request.exact => paths
            .iter()
            .map(|path| names.resolve(path))
            .collect::<Result<std::collections::BTreeSet<_>, _>>()
            .map(|paths| paths.into_iter().collect()),
        Some(paths) => expand(&store.index, &names, paths, request.target),
        None if request.exact => Err("exact needs selected paths".into()),
        None => {
            let mut selected = std::collections::BTreeSet::new();
            let recovery = recovery_reference(&store.index, request.target);
            let has_complete_roots =
                recovery && !recovery::roots(&store.index, request.target).is_empty();
            for path in store.index.paths.keys() {
                let raw_target =
                    recovery && recovery::target(&store.index, request.target, path).is_some();
                // Without complete directory roots, only explicitly saved
                // names belong to this recovery target. Resolve them after
                // filtering so unrelated missing Unicode names cannot make a
                // full restore ambiguous. A root can cover an alias whose raw
                // spelling is absent from the target; keep resolving those.
                if recovery && !has_complete_roots && !raw_target {
                    continue;
                }
                let path = match names.resolve(path) {
                    Ok(path) => path,
                    Err(_) if recovery && !raw_target => continue,
                    Err(error) => return Err(error),
                };
                if !recovery || recovery::target(&store.index, request.target, &path).is_some() {
                    selected.insert(path);
                }
            }
            Ok(selected.into_iter().collect())
        }
    }
}

pub(super) fn check_retained(index: &Index, seq: u64) -> Result<(), String> {
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
pub(super) fn created_after(index: &Index, path: &str, target: u64) -> bool {
    index
        .paths
        .get(path)
        .and_then(|versions| versions.first())
        .is_some_and(|first| first.seq > target && index.coverage_at(target))
}

pub(super) fn expectation(
    index: &Index,
    path: &str,
    target: u64,
    request: &Request<'_>,
) -> (u64, State) {
    let at = |seq: u64| {
        let state = if recovery_reference(index, seq) {
            recovery::target(index, seq, path)
        } else {
            index
                .at(path, seq)
                .map(|version| version.state.clone())
                .or_else(|| created_after(index, path, seq).then_some(State::Absent))
        };
        (
            seq,
            state.unwrap_or_else(|| State::Unreadable {
                reason: "the expected state is no longer recorded".into(),
            }),
        )
    };
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

pub fn recovery_reference(index: &Index, target: u64) -> bool {
    index.recovery.groups.contains_key(&target) || index.recovery.snapshots.contains_key(&target)
}

pub fn target_state(index: &Index, target: u64, path: &str) -> Option<State> {
    if recovery_reference(index, target) {
        recovery::target(index, target, path)
    } else {
        Some(index.state_at(path, target))
    }
}

pub(super) fn same_bytes(store: &Store, a: &State, b: &State) -> Result<bool, String> {
    match (a, b) {
        (State::File { object: x, .. }, State::File { object: y, .. }) if x != y => {
            store.objects_equal(*x, *y)
        }
        _ => Ok(true),
    }
}

/// Selected directories include their recorded descendants.
fn expand(
    index: &Index,
    names: &super::TargetNames,
    paths: &[String],
    target: u64,
) -> Result<Vec<String>, String> {
    let mut expanded = std::collections::BTreeSet::new();
    for path in paths {
        let path = names.resolve(path)?;
        let prefix = format!("{path}/").to_lowercase();
        for (candidate, versions) in &index.paths {
            if !candidate.to_lowercase().starts_with(&prefix) {
                continue;
            }
            let candidate = names.resolve(candidate)?;
            if !candidate.starts_with(&format!("{path}/")) {
                continue;
            }
            let visible = target_state(index, target, &candidate)
                .is_some_and(|state| state.exists())
                || versions.last().is_some_and(|v| v.state.exists());
            if visible {
                expanded.insert(candidate);
            }
        }
        expanded.insert(path);
    }
    Ok(expanded.into_iter().collect())
}
