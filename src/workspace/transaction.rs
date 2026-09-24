//! Recoverable, ordered publication with no-overwrite renames and retained originals.
use super::{
    Budget, Change, ChangeError, Workspace,
    change::{After, fail},
    platform,
    snapshot::{Snapshot, copy_replacement, matches},
};
use std::{
    fs::File,
    io::{self, Write},
    sync::atomic::{AtomicU64, Ordering},
};

pub struct Applied {
    /// An existing original is kept here; no routine cleanup removes it.
    pub recovery: Option<String>,
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
fn unique(kind: &str) -> String {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    format!(
        ".jecode-{kind}-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    )
}
impl Workspace {
    /// The controller calls this only after an explicit decision for this exact
    /// owned Change. Consuming it prevents accidental double application.
    pub fn apply(&self, change: Change, budget: &Budget<'_>) -> Result<Applied, ChangeError> {
        self.apply_with(change, budget, || {})
    }
    fn apply_with(
        &self,
        mut change: Change,
        budget: &Budget<'_>,
        after_stash: impl FnOnce(),
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
        let recovery = if let (Some(original), Some(before)) = (&mut original, &mut change.before) {
            matches(original, before, budget)?;
            let backup = unique("recovery");
            // There is a short absent-name interval, not an atomic replacement.
            // The source inode stays intact; a competing destination wins rather
            // than being overwritten. On any failure restore without replacing.
            platform::move_new(&parent.file, original, &name, &backup)?;
            after_stash();
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
                return Err(ChangeError(if restored {
                    format!("{error}; original restored; proposed content not published")
                } else {
                    format!("{error}; original retained at {location}; destination not overwritten")
                }));
            }
            Some(recovery_path(&path, &backup))
        } else {
            platform::move_new(&parent.file, &staging.file, &staging.name, &name)?;
            None
        };
        staging.published = true;
        // Cancellation after publication cannot turn a real change into an
        // invented 'not executed' receipt. The controller records this outcome.
        Ok(Applied { recovery })
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
