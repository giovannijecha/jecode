//! Stage the desired content before checking the live path immediately before
//! installation. Leases coordinate Jecode writers; external writers can still
//! race the final check and filesystem operation.
use super::{Planned, State, files};
use crate::checkpoints::capture::{self, Observed};
use crate::checkpoints::store::Store;
use std::path::Path;

pub(super) enum Failure {
    Changed(Box<Observed>),
    Io(String),
}

impl From<std::io::Error> for Failure {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error.to_string())
    }
}

impl From<String> for Failure {
    fn from(error: String) -> Self {
        Self::Io(error)
    }
}

pub(super) enum Step {
    Applied,
    Failed,
    Untouched,
}

pub(super) fn save_conflict(store: &mut Store, observed: Observed) -> Result<u64, String> {
    let path = observed.path.clone();
    store.transaction(true, |transaction| {
        let recorded = capture::record(transaction, observed, "external", 0)?;
        Ok(recorded.map_or_else(
            || {
                transaction
                    .index()
                    .current(&path)
                    .map_or(transaction.index().latest(), |version| version.seq)
            },
            |recorded| recorded.seq,
        ))
    })
}

pub(super) fn install(
    store: &Store,
    root: &Path,
    planned: &Planned,
    maximum: u64,
) -> Result<(), Failure> {
    files::confined_parent(root, &planned.path)?;
    let path = crate::checkpoints::identity::absolute(root, &planned.path);
    let pending = match planned.target {
        State::File { object, mode, .. } => {
            Some(files::stage_reader(&path, store.reader(object)?, mode)?)
        }
        _ => None,
    };
    #[cfg(test)]
    crate::checkpoints::tests::process::stage("restore-before-install");
    let observed = capture::observe(root, &planned.path, maximum);
    let matches = capture::matches_state_exact(store, &observed, &planned.current, true)?;
    if !matches || !capture::still_current(root, &observed) {
        // Read the new state when the final metadata check detected a race;
        // saving the earlier observation would misidentify the conflict.
        let observed = capture::observe(root, &planned.path, maximum);
        return Err(Failure::Changed(Box::new(observed)));
    }
    #[cfg(test)]
    crate::checkpoints::tests::process::stage("restore-after-check");
    match &planned.target {
        State::Absent => files::remove(&path)?,
        State::File { .. } => {
            if matches!(
                planned.current,
                State::Link { .. } | State::Directory { .. }
            ) {
                files::remove(&path)?;
            }
            let pending = pending.expect("file target was staged");
            #[cfg(unix)]
            pending.install_deferred()?;
            #[cfg(not(unix))]
            pending.install()?;
        }
        State::Directory { mode } => {
            if !matches!(planned.current, State::Directory { .. }) {
                if !matches!(planned.current, State::Absent) {
                    files::remove(&path)?;
                }
                std::fs::create_dir(&path)?;
                files::sync_directory(path.parent().unwrap_or(root))?;
            }
            #[cfg(unix)]
            files::directory_mode(&path, *mode)?;
            #[cfg(not(unix))]
            let _ = mode;
        }
        State::Link { target } => {
            files::remove(&path)?;
            files::symlink(target, &path)?;
        }
        State::Oversized { .. } | State::Unreadable { .. } | State::UnretainedFile { .. } => {
            return Err(Failure::Io("the target state was not captured".into()));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
