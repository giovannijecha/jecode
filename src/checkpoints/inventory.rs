//! Bounded discovery. An oversized scope is rejected before copying an
//! arbitrary prefix of its files into the archive.
use super::policy::Policy;
use super::record::Stat;
use super::scan::{WORKERS, in_scope};
use std::fs;
use std::path::Path;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Condvar, Mutex};

pub const MAX_PATHS: usize = 50_000;
const PATH_BYTES: u64 = 512;

#[derive(Clone, Copy)]
pub struct Limits {
    pub paths: usize,
    pub bytes: u64,
}

impl Limits {
    pub fn for_policy(policy: &Policy) -> Self {
        Self {
            paths: MAX_PATHS,
            bytes: policy.budget_bytes / 4,
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
        if self
            .bytes
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |used| {
                used.checked_add(length + PATH_BYTES)
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

pub fn bounded(
    root: &Path,
    start: &str,
    policy: &Policy,
    excluded: &[String],
    limits: Limits,
) -> Walk {
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
                            state = ready.wait(state).unwrap_or_else(|e| e.into_inner());
                        }
                    };
                    let Some(directory) = directory else {
                        ready.notify_all();
                        break;
                    };
                    let found = read(root, &directory, policy, excluded, &budget, &mut local);
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
    walk
}

fn read(
    root: &Path,
    directory: &str,
    policy: &Policy,
    excluded: &[String],
    budget: &Budget,
    walk: &mut Walk,
) -> Vec<String> {
    let mut found = Vec::new();
    let Ok(entries) = fs::read_dir(super::identity::absolute(root, directory)) else {
        walk.unreadable += 1;
        walk.unreadable_dirs.push(directory.into());
        return found;
    };
    for entry in entries {
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
        let directory = kind.as_ref().is_ok_and(|kind| kind.is_dir());
        if !in_scope(policy, excluded, &path, directory) {
            walk.excluded += 1;
            continue;
        }
        let metadata = entry.metadata();
        let stat = metadata.as_ref().map_or(Stat::default(), Stat::of);
        let length = if !directory
            && !kind.as_ref().is_ok_and(|kind| kind.is_symlink())
            && stat.length <= policy.max_file_bytes
        {
            stat.length
        } else {
            0
        };
        if !budget.admit(length + path.len() as u64 * 2) {
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
