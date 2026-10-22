//! Scope discovery. Explicit limits remain available for isolated fixtures.
use super::policy::Policy;
use super::record::Stat;
use super::scan::{WORKERS, in_scope};
use crate::cancel::Cancellation;
use std::fs;
use std::path::Path;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Condvar, Mutex};
use std::time::Duration;

const PATH_BYTES: u64 = 512;

/// Reparse directories (including Windows junctions) are never traversed.
pub fn ordinary_directory(metadata: &fs::Metadata) -> bool {
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return false;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const REPARSE_POINT: u32 = 0x400;
        metadata.file_attributes() & REPARSE_POINT == 0
    }
    #[cfg(not(windows))]
    {
        true
    }
}

#[derive(Clone, Copy)]
pub struct Limits {
    pub paths: usize,
    pub bytes: u64,
}

impl Limits {
    pub fn for_policy(_policy: &Policy) -> Self {
        Self {
            paths: usize::MAX,
            bytes: u64::MAX,
        }
    }
}

pub struct Entry {
    pub path: String,
    pub directory: bool,
    pub stat: Stat,
}

#[derive(Default)]
pub struct Walk {
    pub entries: Vec<Entry>,
    pub excluded: usize,
    pub unreadable: usize,
    pub unreadable_dirs: Vec<String>,
    pub unsupported: Vec<String>,
    pub limited: Option<String>,
}

struct Budget {
    limits: Limits,
    paths: AtomicUsize,
    bytes: AtomicU64,
    stopped: AtomicUsize,
}

impl Budget {
    fn visit(&self) -> bool {
        if self.stopped.load(Ordering::Relaxed) != 0 {
            return false;
        }
        if self.paths.fetch_add(1, Ordering::Relaxed) >= self.limits.paths {
            self.stopped.store(1, Ordering::Relaxed);
            return false;
        }
        true
    }

    fn admit(&self, length: u64) -> bool {
        if self.limits.bytes == u64::MAX {
            return true;
        }
        if self
            .bytes
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |used| {
                used.checked_add(length.saturating_add(PATH_BYTES))
                    .filter(|total| *total <= self.limits.bytes)
            })
            .is_err()
        {
            self.stopped.store(2, Ordering::Relaxed);
            return false;
        }
        true
    }
}

pub fn walk(root: &Path, start: &str, policy: &Policy, excluded: &[String]) -> Walk {
    bounded(root, start, policy, excluded, Limits::for_policy(policy))
}

pub fn walk_cancellable(
    root: &Path,
    start: &str,
    policy: &Policy,
    excluded: &[String],
    cancellation: &Cancellation,
) -> Result<Walk, String> {
    bounded_cancellable(
        root,
        start,
        policy,
        excluded,
        Limits::for_policy(policy),
        cancellation,
    )
}

pub fn bounded(
    root: &Path,
    start: &str,
    policy: &Policy,
    excluded: &[String],
    limits: Limits,
) -> Walk {
    // The non-cancellable entry point is retained for existing callers.
    bounded_cancellable(
        root,
        start,
        policy,
        excluded,
        limits,
        &Cancellation::default(),
    )
    .expect("an uncancelled inventory cannot be cancelled")
}

pub fn bounded_cancellable(
    root: &Path,
    start: &str,
    policy: &Policy,
    excluded: &[String],
    limits: Limits,
    cancellation: &Cancellation,
) -> Result<Walk, String> {
    struct Queue {
        directories: Vec<String>,
        active: usize,
    }
    let queue = Mutex::new(Queue {
        directories: vec![start.into()],
        active: 0,
    });
    let ready = Condvar::new();
    let results = Mutex::new(Walk::default());
    let budget = Budget {
        limits,
        paths: AtomicUsize::new(0),
        bytes: AtomicU64::new(0),
        stopped: AtomicUsize::new(0),
    };
    std::thread::scope(|scope| {
        for _ in 0..WORKERS {
            scope.spawn(|| {
                let mut local = Walk::default();
                loop {
                    let directory = {
                        let mut state = queue.lock().unwrap_or_else(|e| e.into_inner());
                        loop {
                            if cancellation.requested() {
                                budget.stopped.store(3, Ordering::Relaxed);
                            }
                            if budget.stopped.load(Ordering::Relaxed) != 0 {
                                break None;
                            }
                            if let Some(directory) = state.directories.pop() {
                                state.active += 1;
                                break Some(directory);
                            }
                            if state.active == 0 {
                                break None;
                            }
                            state = ready
                                .wait_timeout(state, Duration::from_millis(50))
                                .unwrap_or_else(|e| e.into_inner())
                                .0;
                        }
                    };
                    let Some(directory) = directory else {
                        ready.notify_all();
                        break;
                    };
                    let found = read(
                        root,
                        &directory,
                        policy,
                        excluded,
                        &budget,
                        cancellation,
                        &mut local,
                    );
                    let mut state = queue.lock().unwrap_or_else(|e| e.into_inner());
                    state.directories.extend(found);
                    state.active -= 1;
                    ready.notify_all();
                }
                let mut results = results.lock().unwrap_or_else(|e| e.into_inner());
                results.entries.append(&mut local.entries);
                results.excluded += local.excluded;
                results.unreadable += local.unreadable;
                results.unreadable_dirs.append(&mut local.unreadable_dirs);
                results.unsupported.append(&mut local.unsupported);
            });
        }
    });
    if cancellation.requested() || budget.stopped.load(Ordering::Relaxed) == 3 {
        return Err("Checkpoint capture was cancelled".into());
    }
    let mut walk = results.into_inner().unwrap_or_else(|e| e.into_inner());
    walk.entries.sort_by(|a, b| a.path.cmp(&b.path));
    walk.limited = match budget.stopped.load(Ordering::Relaxed) {
        1 => Some(format!("discovery exceeds {} paths", limits.paths)),
        2 => Some(format!(
            "baseline exceeds {} of content and path metadata",
            super::policy::size(limits.bytes)
        )),
        _ => None,
    };
    Ok(walk)
}

fn read(
    root: &Path,
    directory: &str,
    policy: &Policy,
    excluded: &[String],
    budget: &Budget,
    cancellation: &Cancellation,
    walk: &mut Walk,
) -> Vec<String> {
    let mut found = Vec::new();
    if !directory.is_empty()
        && (super::files::confined_parent(root, directory).is_err()
            || !fs::symlink_metadata(super::identity::absolute(root, directory))
                .is_ok_and(|metadata| ordinary_directory(&metadata)))
    {
        walk.unreadable += 1;
        walk.unreadable_dirs.push(directory.into());
        return found;
    }
    let Ok(entries) = fs::read_dir(super::identity::absolute(root, directory)) else {
        walk.unreadable += 1;
        walk.unreadable_dirs.push(directory.into());
        return found;
    };
    for entry in entries {
        if cancellation.requested() {
            budget.stopped.store(3, Ordering::Relaxed);
            break;
        }
        if !budget.visit() {
            break;
        }
        let entry = match entry {
            Ok(entry) => entry,
            Err(_) => {
                walk.unreadable += 1;
                walk.unreadable_dirs.push(directory.into());
                continue;
            }
        };
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            // Even unsupported names consume discovery capacity.
            if !budget.admit((directory.len() + name.len()) as u64 * 2) {
                break;
            }
            walk.unsupported
                .push(format!("{directory}/{}", name.to_string_lossy()));
            continue;
        };
        let path = if directory.is_empty() {
            name.into()
        } else {
            format!("{directory}/{name}")
        };
        let kind = entry.file_type();
        let metadata = fs::symlink_metadata(entry.path());
        let directory = kind
            .as_ref()
            .is_ok_and(|kind| kind.is_dir() && !kind.is_symlink())
            && metadata.as_ref().is_ok_and(ordinary_directory);
        if !in_scope(policy, excluded, &path, directory) {
            walk.excluded += 1;
            continue;
        }
        let stat = metadata.as_ref().map_or(Stat::default(), Stat::of);
        let length = if !directory
            && !kind.as_ref().is_ok_and(|kind| kind.is_symlink())
            && stat.length <= policy.max_file_bytes
        {
            stat.length
        } else {
            0
        };
        if !budget.admit(length.saturating_add((path.len() as u64).saturating_mul(2))) {
            break;
        }
        if kind.is_err() || metadata.is_err() {
            walk.unreadable += 1;
            walk.unreadable_dirs.push(if directory {
                path.clone()
            } else {
                path.rsplit_once('/')
                    .map_or("", |(parent, _)| parent)
                    .into()
            });
        } else if directory {
            found.push(path.clone());
        }
        walk.entries.push(Entry {
            path,
            directory,
            stat,
        });
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancelled_walk_returns_no_partial_inventory() {
        let root = crate::test_support::Directory::new();
        std::fs::create_dir(root.path().join("nested")).unwrap();
        let cancellation = Cancellation::default();
        cancellation.cancel();
        let result = walk_cancellable(root.path(), "", &Policy::default(), &[], &cancellation);
        assert!(result.err().unwrap().contains("cancelled"));
    }
}
