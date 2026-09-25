//! Recoverable, ordered publication with a private durable copy and transient adjacent names.
use super::{
    Budget, Change, ChangeError, RecoveryStore, Workspace,
    change::{After, fail},
    platform,
    recovery::{Origin, same},
    snapshot::{Snapshot, copy_replacement, matches},
};
use std::{
    fs::File,
    io::{self, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

pub struct Applied {
    /// Stable identifier of a private retained version, not a workspace path.
    pub recovery: Option<String>,
    pub warning: Option<String>,
    pub stop_after: bool,
}
struct Staging<'a> {
    parent: &'a File,
    file: File,
    name: String,
    published: bool,
}
impl Drop for Staging<'_> {
    fn drop(&mut self) {
        if !self.published {
            let _ = platform::remove_owned(self.parent, &self.file, &self.name);
        }
    }
}
pub(super) fn unique(kind: &str) -> String {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    format!(
        ".jecode-{kind}-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    )
}
impl Workspace {
    /// The controller consumes this exact prepared Change once after its durable
    /// pre-effect checkpoint. Consuming it prevents accidental double application.
    pub fn apply(
        &self,
        change: Change,
        budget: &Budget<'_>,
        recoveries: &RecoveryStore,
        session: Option<&str>,
        operation: &str,
    ) -> Result<Applied, ChangeError> {
        self.apply_with(
            change,
            budget,
            recoveries,
            session,
            operation,
            (|| {}, || {}),
        )
    }
    fn apply_with(
        &self,
        mut change: Change,
        budget: &Budget<'_>,
        recoveries: &RecoveryStore,
        session: Option<&str>,
        operation: &str,
        hooks: (impl FnOnce(), impl FnOnce()),
    ) -> Result<Applied, ChangeError> {
        let (path, parent, name) = self.change_parent(&change.preview.path, budget)?;
        if platform::identity(&parent.file)? != change.parent {
            return fail("parent directory changed since the preview; nothing published");
        }
        let mut original = if let Some(before) = &mut change.before {
            let mut file = platform::edit_open(&parent.file, &name)?;
            platform::editable(&file)?;
            matches(&mut file, before, budget)?;
            Some(file)
        } else {
            match platform::edit_open(&parent.file, &name) {
                Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                _ => return fail("file appeared since the create preview; nothing published"),
            }
            None
        };
        let stage_name = unique("staging");
        let mut staging = Staging {
            parent: &parent.file,
            file: platform::create(&parent.file, &stage_name)?,
            name: stage_name,
            published: false,
        };
        // Install the original access policy before writing any proposed bytes.
        if let Some(original) = &original {
            platform::metadata_to(original, &staging.file)?;
        }
        match &change.after {
            After::Complete(after) => {
                for chunk in after.as_bytes().chunks(16384) {
                    budget.check()?;
                    staging.file.write_all(chunk)?;
                }
            }
            After::Replacement { at, old_len, new } => {
                copy_replacement(
                    change.before.as_mut().unwrap(),
                    &mut staging.file,
                    *at,
                    *old_len,
                    new,
                    budget,
                )?;
            }
        }
        staging.file.sync_all()?;
        budget.check()?;
        let (_, current_parent, _) = self.change_parent(&path, budget)?;
        if platform::identity(&current_parent.file)? != change.parent {
            return fail("parent directory changed during preparation; nothing published");
        }
        drop(current_parent);
        let mut warning = None;
        let mut stop_after = false;
        let recovery = if let (Some(original), Some(before)) = (&mut original, &mut change.before) {
            matches(original, before, budget)?;
            let backup = unique("recovery");
            let target = absolute(self.path(), &path);
            let adjacent = target.with_file_name(&backup);
            // Copy across volumes in bounded chunks and sync both private files
            // and their manifest before removing the target's original name.
            let mut version = recoveries
                .capture(
                    Origin {
                        workspace: self.path(),
                        target: &target,
                        session,
                        operation,
                        adjacent: &adjacent,
                    },
                    original,
                    &mut staging.file,
                    budget,
                )
                .map_err(|error| {
                    ChangeError(
                        format!(
                            "could not durably retain the original before publication: {error}"
                        ),
                        error.id,
                    )
                })?;
            matches(original, before, budget)
                .map_err(|error| ChangeError(error.0, Some(version.id.clone())))?;
            let mut expected_after = recoveries
                .file(&version.id, "after")
                .map_err(|error| ChangeError(error.to_string(), Some(version.id.clone())))?;
            let stage_matches = same(&mut staging.file, &mut expected_after, budget)
                .map_err(|error| ChangeError(error.to_string(), Some(version.id.clone())))?;
            if !stage_matches {
                return Err(ChangeError(
                    "staged result changed before publication; original remains in place".into(),
                    Some(version.id),
                ));
            }
            // There is a short absent-name interval, not an atomic replacement.
            // The source inode stays intact; a competing destination wins rather
            // than being overwritten. On any failure restore without replacing.
            platform::move_new(&parent.file, original, &name, &backup).map_err(|error| {
                ChangeError(
                    format!("could not begin publication: {error}; original retained at target"),
                    Some(version.id.clone()),
                )
            })?;
            (hooks.0)();
            // Linux renames by name, Windows by held handle. Reopen the moved
            // object on Linux to detect a path replacement during the first move.
            let validation = validate_stash(&parent.file, original, &backup, before, budget);
            let publish = validation.and_then(|()| {
                budget.check()?;
                let (_, current_parent, _) = self.change_parent(&path, budget)?;
                if platform::identity(&current_parent.file)? != change.parent {
                    return fail("parent directory changed during publication");
                }
                platform::move_new(&parent.file, &staging.file, &staging.name, &name)
                    .map_err(ChangeError::from)
            });
            if let Err(error) = publish {
                let restored = platform::move_new(&parent.file, original, &backup, &name).is_ok();
                let location = recovery_path(&path, &backup);
                return Err(ChangeError(
                    if restored {
                        format!("{error}; original restored; proposed content not published")
                    } else {
                        format!(
                            "{error}; original retained at {location}; destination not overwritten"
                        )
                    },
                    Some(version.id),
                ));
            }
            staging.published = true;
            (hooks.1)();
            let checkpoint = platform::sync_parent(&parent.file)
                .and_then(|()| platform::identity(&staging.file))
                .and_then(|identity| recoveries.record(&mut version, "applied", Some(identity)));
            match checkpoint {
                Ok(()) => {}
                Err(error) => {
                    warning = Some(format!(
                        "change applied but recovery result checkpoint failed: {error}; adjacent transient retained at {}",
                        adjacent.display()
                    ));
                    stop_after = true;
                }
            }
            if !stop_after {
                let cleanup = platform::remove_owned(&parent.file, original, &backup)
                    .and_then(|()| platform::sync_parent(&parent.file));
                if let Err(error) = cleanup {
                    warning = Some(format!(
                        "change applied; adjacent cleanup or directory sync failed at {}: {error}",
                        adjacent.display()
                    ));
                    stop_after = true;
                }
            }
            Some(version.id)
        } else {
            platform::move_new(&parent.file, &staging.file, &staging.name, &name)?;
            staging.published = true;
            if let Err(error) = platform::sync_parent(&parent.file) {
                warning = Some(format!("file created but directory sync failed: {error}"));
                stop_after = true;
            }
            None
        };
        staging.published = true;
        // Cancellation after publication cannot turn a real change into an
        // invented 'not executed' receipt. The controller records this outcome.
        Ok(Applied {
            recovery,
            warning,
            stop_after,
        })
    }
}
fn absolute(workspace: &Path, path: &str) -> PathBuf {
    let target = Path::new(path);
    if target.is_absolute() {
        target.to_owned()
    } else {
        workspace.join(target)
    }
}
fn recovery_path(path: &str, backup: &str) -> String {
    path.rsplit_once('/')
        .map_or_else(|| backup.into(), |(parent, _)| format!("{parent}/{backup}"))
}
fn validate_stash(
    parent: &File,
    original: &mut File,
    backup: &str,
    before: &mut Snapshot,
    budget: &Budget<'_>,
) -> Result<(), ChangeError> {
    #[cfg(not(windows))]
    {
        let mut moved = platform::edit_open(parent, backup)?;
        matches(&mut moved, before, budget)?;
    }
    #[cfg(windows)]
    let _ = (parent, backup);
    matches(original, before, budget)
}

#[cfg(test)]
#[path = "transaction_tests.rs"]
mod tests;
