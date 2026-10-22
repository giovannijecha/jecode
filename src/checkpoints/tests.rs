//! Deterministic checks of the checkpoint core on isolated synthetic
//! workspaces and archive homes.
use super::index::Index;
use super::restore::{Expected, Report, Request};
use super::workspace::{self, Options, Workspace};
use crate::test_support::Directory;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

mod command;
mod gc;
mod history;
mod journal_limits;
mod maintenance;
mod mutation;
mod notices;
pub(crate) mod process;
mod recovery;
mod restore;
#[cfg(unix)]
mod restore_links;
mod restore_races;
mod scope;
mod selective;
mod streaming;
#[cfg(windows)]
mod watch;

pub(super) struct Fixture {
    pub home: Directory,
    pub root: Directory,
}

impl Fixture {
    pub fn new() -> Self {
        Self {
            home: Directory::new(),
            root: Directory::new(),
        }
    }

    pub fn path(&self, relative: &str) -> PathBuf {
        super::identity::absolute(self.root.path(), relative)
    }

    pub fn write(&self, relative: &str, bytes: &[u8]) {
        let path = self.path(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        set_readonly(&path, false);
        fs::write(path, bytes).unwrap();
    }

    pub fn read(&self, relative: &str) -> Option<Vec<u8>> {
        fs::read(self.path(relative)).ok()
    }

    /// Opens without a watcher or worker and captures the scope on this thread.
    pub fn open(&self) -> Arc<Workspace> {
        self.open_with(Options::offline())
    }

    pub fn open_with(&self, options: Options) -> Arc<Workspace> {
        let workspace = workspace::open(self.home.path(), self.root.path(), options).unwrap();
        if !options.background {
            workspace.initialize().unwrap();
        }
        workspace
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        // Read-only files must not keep the fixture directory alive.
        clear_readonly(self.root.path());
    }
}

fn clear_readonly(path: &std::path::Path) {
    let Ok(entries) = fs::read_dir(path) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        match entry.file_type() {
            Ok(kind) if kind.is_dir() => clear_readonly(&path),
            Ok(kind) if kind.is_file() => set_readonly(&path, false),
            _ => {}
        }
    }
}

pub(super) fn set_readonly(path: &std::path::Path, readonly: bool) {
    if let Ok(metadata) = fs::metadata(path) {
        let mut permissions = metadata.permissions();
        #[allow(clippy::permissions_set_readonly_false)]
        permissions.set_readonly(readonly);
        let _ = fs::set_permissions(path, permissions);
    }
}

/// Deterministic bytes that compress poorly, so deltas and budgets are real.
pub(super) fn noise(seed: u64, length: usize) -> Vec<u8> {
    let mut state = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
    (0..length)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state as u8
        })
        .collect()
}

pub(super) fn request<'a>(
    workspace: &'a Workspace,
    target: u64,
    paths: Option<&'a [String]>,
    expected: Expected,
) -> Request<'a> {
    Request {
        target,
        paths,
        exact: false,
        expected,
        preview: false,
        origin: "test",
        excluded: &workspace.excluded,
    }
}

pub(super) fn restored(report: &Report) -> Vec<String> {
    report
        .applied
        .iter()
        .map(|(path, _)| path.clone())
        .collect()
}

pub(super) fn index(workspace: &Workspace) -> Index {
    let mut store = workspace.store();
    store.refresh().unwrap();
    store.index.clone()
}

pub(super) fn paths(list: &[&str]) -> Vec<String> {
    list.iter().map(|path| path.to_string()).collect()
}
