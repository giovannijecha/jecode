//! Workspace-relative names and bounded, explicit scope discovery.
use super::{Issue, Snapshot, files};
use crate::cancel::Cancellation;
use crate::disk::ordinary_directory as ordinary_dir;
use std::collections::BTreeSet;
use std::fs;
use std::path::{Component, Path, PathBuf};

#[derive(Default, Debug)]
pub struct Selection {
    pub paths: Vec<String>,
    /// Directory roots whose eligible descendants were completely visited.
    pub roots: Vec<String>,
    /// Directories a walk skipped because Git ignores them.
    pub ignored: Vec<String>,
    pub ignore_rules: IgnoreRules,
    pub partial: Vec<Issue>,
}

pub fn absolute(root: &Path, relative: &str) -> PathBuf {
    relative
        .split('/')
        .filter(|part| !part.is_empty())
        .fold(root.to_path_buf(), |path, part| path.join(part))
}

pub(super) fn normalized_request(request: &str) -> String {
    #[cfg(windows)]
    let request = request.replace('\\', "/");
    #[cfg(not(windows))]
    let request = request.to_owned();
    let trimmed = request.trim_end_matches('/');
    if trimmed.is_empty()
        || (Path::new(&request).is_absolute() && !Path::new(trimmed).is_absolute())
    {
        request
    } else {
        trimmed.to_owned()
    }
}

/// Resolve ancestors but retain the final directory entry, including a link.
/// `root` must be canonical, as the store's root is.
pub fn relative(root: &Path, path: &Path) -> Option<String> {
    let mut missing = vec![path.file_name()?.to_os_string()];
    if let Some(spelling) = existing_name(path) {
        missing[0] = spelling;
    }
    let mut parent = path.parent()?;
    let resolved = loop {
        if let Ok(resolved) = fs::canonicalize(parent) {
            break resolved;
        }
        missing.push(parent.file_name()?.to_os_string());
        parent = parent.parent()?;
    };
    let path = missing
        .into_iter()
        .rev()
        .fold(resolved, |path, name| path.join(name));
    let rest = path.strip_prefix(root).ok()?;
    let parts: Option<Vec<_>> = rest
        .components()
        .map(|part| match part {
            Component::Normal(name) => name.to_str().map(str::to_owned),
            _ => None,
        })
        .collect();
    let parts = parts?;
    (!parts.is_empty()).then(|| parts.join("/"))
}

fn existing_name(path: &Path) -> Option<std::ffi::OsString> {
    let metadata = fs::symlink_metadata(path).ok()?;
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const REPARSE_POINT: u32 = 0x400;
        if metadata.file_attributes() & REPARSE_POINT != 0 {
            let requested = path.file_name()?;
            let folded = requested.to_string_lossy().to_lowercase();
            let mut found = None;
            for entry in fs::read_dir(path.parent()?).ok()?.flatten() {
                let name = entry.file_name();
                if name == requested {
                    return Some(name);
                }
                if name.to_string_lossy().to_lowercase() == folded {
                    if found.is_some() {
                        return None;
                    }
                    found = Some(name);
                }
            }
            return found;
        }
    }
    if metadata.file_type().is_symlink() {
        return None;
    }
    fs::canonicalize(path)
        .ok()?
        .file_name()
        .map(std::ffi::OsStr::to_os_string)
}

/// Match a saved name without reading or hashing current file contents. An
/// exact saved spelling remains readable after its live entry was deleted.
pub fn resolve_saved(root: &Path, snapshot: &Snapshot, requested: &str) -> Result<String, String> {
    let normalized = normalized_request(requested);
    let requested = normalized.as_str();
    if Path::new(requested)
        .components()
        .any(|part| part == Component::ParentDir)
    {
        return Err(format!("Invalid workspace path: {requested}"));
    }
    let lexical = if Path::new(requested).is_absolute() {
        let rest = Path::new(requested)
            .strip_prefix(root)
            .map_err(|_| format!("Path outside workspace: {requested}"))?;
        let parts: Option<Vec<_>> = rest
            .components()
            .map(|part| match part {
                Component::Normal(name) => name.to_str().map(str::to_owned),
                _ => None,
            })
            .collect();
        parts
            .ok_or_else(|| format!("Invalid workspace path: {requested}"))?
            .join("/")
    } else {
        requested.to_owned()
    };
    if lexical.is_empty() || !lexical.split('/').all(files::normal_component) {
        return Err(format!("Invalid workspace path: {requested}"));
    }
    if snapshot.entries.contains_key(&lexical) {
        return Ok(lexical);
    }
    if let Some(live) = relative(root, &absolute(root, &lexical))
        && snapshot.entries.contains_key(&live)
    {
        return Ok(live);
    }
    if let Some(saved) = saved_alias(root, snapshot, &lexical) {
        return Ok(saved);
    }
    Err(format!("Path was not saved by this call: {requested}"))
}

/// An existing live entry can represent a differently cased saved spelling on
/// a case-insensitive volume. Require a single directory entry, and verify
/// that the saved spelling names it on this filesystem. Distinct hardlinks or
/// case-distinct directories on a case-sensitive volume remain separate.
pub fn saved_alias(root: &Path, snapshot: &Snapshot, live: &str) -> Option<String> {
    if snapshot.entries.contains_key(live) {
        return Some(live.to_owned());
    }
    let live_path = absolute(root, live);
    fs::symlink_metadata(&live_path).ok()?;
    let live_parent = live_path.parent()?;
    let live_name = live_path.file_name()?.to_str()?;
    let folded = live.to_lowercase();
    let mut found = None;
    for saved in snapshot
        .entries
        .keys()
        .filter(|name| name.to_lowercase() == folded)
    {
        let saved_path = absolute(root, saved);
        if fs::symlink_metadata(&saved_path).is_err()
            || !same_directory(saved_path.parent()?, live_parent)
        {
            continue;
        }
        let mut names = fs::read_dir(live_parent)
            .ok()?
            .filter_map(|entry| entry.ok()?.file_name().into_string().ok())
            .filter(|name| name.to_lowercase() == live_name.to_lowercase());
        if names.next().as_deref() != Some(live_name) || names.next().is_some() {
            continue;
        }
        if found.is_some() {
            return None;
        }
        found = Some(saved.clone());
    }
    found
}

fn same_directory(a: &Path, b: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        fs::metadata(a)
            .ok()
            .zip(fs::metadata(b).ok())
            .is_some_and(|(a, b)| a.dev() == b.dev() && a.ino() == b.ino())
    }
    #[cfg(not(unix))]
    {
        fs::canonicalize(a)
            .ok()
            .zip(fs::canonicalize(b).ok())
            .is_some_and(|(a, b)| a == b)
    }
}

fn protected(path: &str, excluded: &[String]) -> bool {
    !excluded.iter().any(|item| {
        #[cfg(windows)]
        let prefix = path
            .get(..item.len())
            .is_some_and(|part| part.eq_ignore_ascii_case(item));
        #[cfg(not(windows))]
        let prefix = path.starts_with(item);
        item.is_empty()
            || prefix
                && (path.len() == item.len() || path.as_bytes().get(item.len()) == Some(&b'/'))
    })
}

const GENERATED: &[&str] = &[
    ".git",
    ".hg",
    ".svn",
    ".jj",
    ".bzr",
    "_darcs",
    ".jecode-output",
    "target",
    "node_modules",
    "__pycache__",
    ".venv",
    ".mypy_cache",
    ".pytest_cache",
    ".ruff_cache",
    ".tox",
    ".gradle",
    ".next",
    ".nuxt",
    ".turbo",
    ".parcel-cache",
    ".svelte-kit",
    ".terraform",
    ".cache",
];

/// Eligibility for an implicit recursive walk below `selected_root`. Explicit
/// paths only need `protected`.
pub(super) fn eligible_beneath(path: &str, selected_root: &str, excluded: &[String]) -> bool {
    let below = if selected_root.is_empty() {
        path
    } else {
        path.strip_prefix(selected_root)
            .and_then(|rest| rest.strip_prefix('/'))
            .unwrap_or(path)
    };
    protected(path, excluded)
        && !below.split('/').any(|part| {
            GENERATED.iter().any(|name| name.eq_ignore_ascii_case(part))
                || (part.starts_with(".jecode-") && part.ends_with(".tmp"))
        })
}

mod discovery;
pub use discovery::discover;
pub(super) use discovery::discover_saved;

mod exact;
mod ignore;
pub use exact::discover_exact;
pub use ignore::{Ignore, IgnoreRules};

#[cfg(test)]
#[path = "paths/tests.rs"]
mod tests;
