//! Sync shared parent directories in bounded groups before recording restore
//! completion. Flush before final directory modes can remove traversal access.
use super::{Planned, State, files};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

const BATCH: usize = 64;

#[derive(Default)]
pub(super) struct Directories {
    pending: BTreeMap<PathBuf, String>,
}

impl Directories {
    pub(super) fn before(&mut self, planned: &Planned) -> Result<(), (String, String)> {
        if self.pending.len() >= BATCH || matches!(planned.target, State::Directory { .. }) {
            self.flush()?;
        }
        Ok(())
    }

    pub(super) fn changed(&mut self, root: &Path, planned: &Planned) {
        if cfg!(unix) && matches!(planned.target, State::File { .. }) {
            let path = crate::checkpoints::identity::absolute(root, &planned.path);
            self.pending
                .entry(path.parent().unwrap_or(root).to_path_buf())
                .or_insert_with(|| planned.path.clone());
        }
    }

    pub(super) fn flush(&mut self) -> Result<(), (String, String)> {
        let mut failure = None;
        self.pending.retain(|parent, path| {
            match files::sync_directory(parent) {
                Ok(()) => false,
                Err(error) => {
                    failure.get_or_insert_with(|| (
                        path.clone(),
                        format!("could not sync restore directory {}: {error}; restore durability is not established", parent.display()),
                    ));
                    true
                }
            }
        });
        failure.map_or(Ok(()), Err)
    }

    /// An installed rename must remain prepared until its parent is durable.
    pub(super) fn pending(&self, root: &Path, planned: &Planned) -> bool {
        if cfg!(unix) && matches!(planned.target, State::File { .. }) {
            let path = crate::checkpoints::identity::absolute(root, &planned.path);
            self.pending.contains_key(path.parent().unwrap_or(root))
        } else {
            false
        }
    }
}
