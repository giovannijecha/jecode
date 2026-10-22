//! Stage the desired content before checking the live path immediately before
//! installation. Leases coordinate Jecode writers; external writers can still
//! race the final check and filesystem operation.
use super::{Planned, State, files};
use crate::checkpoints::capture::{self, Observation, Observed};
use crate::checkpoints::store::Store;
use std::path::Path;

pub(super) enum Failure {
    Changed(Observed),
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
        State::File { object, mode, .. } => Some(files::stage(&path, &store.bytes(object)?, mode)?),
        _ => None,
    };
    // Load the expected bytes before the live read, so object reconstruction
    // cannot widen the final check-to-install interval.
    let before = match planned.current {
        State::File { object, .. } => Some(store.bytes(object)?),
        _ => None,
    };
    #[cfg(test)]
    crate::checkpoints::tests::process::stage("restore-before-install");
    let observed = capture::observe(root, &planned.path, maximum);
    let matches = match (&planned.current, &observed.kind) {
        (State::Absent, Observation::Absent) => true,
        (State::File { mode, .. }, Observation::File(bytes, current_mode)) => {
            mode == current_mode && before.as_ref().is_some_and(|old| old[..] == bytes[..])
        }
        (State::Directory { mode }, Observation::Directory(current)) => mode == current,
        (State::Link { target }, Observation::Link(current)) => target == current,
        _ => false,
    };
    if !matches {
        return Err(Failure::Changed(observed));
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
        State::Oversized { .. } | State::Unreadable { .. } => {
            return Err(Failure::Io("the target state was not captured".into()));
        }
    }
    Ok(())
}
