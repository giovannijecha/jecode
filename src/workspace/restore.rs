//! Explicit, conflict-checked restoration of a selected retained file version.
//! Inspection, restoration and interrupted-transaction repair share one
//! target validation and no-overwrite boundary, so they stay together here.
use super::{
    Budget, RecoveryStore, Workspace, platform,
    recovery::{Version, same},
    transaction::unique,
};
use std::{
    fs::{File, FileTimes},
    io::{self, Read, Write},
    path::Path,
    time::SystemTime,
};

pub struct Inspection {
    pub version: Version,
    pub target: &'static str,
    pub adjacent: &'static str,
}
pub struct Restored {
    pub warning: Option<String>,
}

struct Stage<'a> {
    parent: &'a File,
    file: File,
    name: String,
    published: bool,
}
impl Drop for Stage<'_> {
    fn drop(&mut self) {
        if !self.published {
            let _ = platform::remove_owned(self.parent, &self.file, &self.name);
        }
    }
}
impl RecoveryStore {
    /// Reconcile a captured/interrupted item without overwriting an occupied name.
    pub fn repair(
        &self,
        workspace: &Workspace,
        id: &str,
        budget: &Budget<'_>,
    ) -> io::Result<String> {
        let mut version = self.get(id)?;
        require_workspace(workspace, &version)?;
        if !matches!(version.state.as_str(), "captured" | "restoring" | "applied") {
            return Err(io::Error::other(
                "version has no incomplete transaction to repair",
            ));
        }
        let (_, parent, name) = workspace
            .change_parent(&version.target, budget)
            .map_err(|_| io::Error::other("recorded target is unavailable"))?;
        if version.state == "applied" {
            let adjacent = cleanup_adjacent(
                self,
                &version,
                &parent.file,
                "before",
                version.original_identity,
                budget,
            )?;
            return Ok(adjacent.into());
        }
        match platform::edit_open(&parent.file, &name) {
            Ok(mut current) => {
                let identity = platform::identity(&current)?;
                if version.state == "captured"
                    && identity == version.staged_identity
                    && matches_saved(
                        self,
                        &version,
                        &mut current,
                        "after",
                        version.after_modified,
                        budget,
                    )?
                {
                    self.record(&mut version, "applied", Some(identity))?;
                    let adjacent = cleanup_adjacent(
                        self,
                        &version,
                        &parent.file,
                        "before",
                        version.original_identity,
                        budget,
                    )?;
                    return Ok(format!(
                        "Confirmed published result; retained original is ready to restore. {adjacent}"
                    ));
                }
                if version.state == "restoring"
                    && version.restore_identity == Some(identity)
                    && matches_content(self, &version, &mut current, "before", budget)?
                {
                    let previous_identity = version
                        .published_identity
                        .ok_or(io::ErrorKind::InvalidData)?;
                    self.record(&mut version, "restored", Some(identity))?;
                    let adjacent = cleanup_adjacent(
                        self,
                        &version,
                        &parent.file,
                        "after",
                        previous_identity,
                        budget,
                    )?;
                    return Ok(format!(
                        "Confirmed restored original; no file was overwritten. {adjacent}"
                    ));
                }
                if version.state == "restoring"
                    && version.published_identity == Some(identity)
                    && matches_saved(
                        self,
                        &version,
                        &mut current,
                        "after",
                        version.after_modified,
                        budget,
                    )?
                {
                    self.record(&mut version, "applied", Some(identity))?;
                    return Ok("Restoration had not published; result remains in place.".into());
                }
                if version.state == "captured"
                    && identity == version.original_identity
                    && matches_saved(
                        self,
                        &version,
                        &mut current,
                        "before",
                        version.before_modified,
                        budget,
                    )?
                {
                    self.record(&mut version, "not_published", None)?;
                    return Ok("Original remained in place; no file was overwritten.".into());
                }
                Err(io::Error::other(
                    "target conflicts with both retained versions; repair refused",
                ))
            }
            Err(error)
                if error.kind() == io::ErrorKind::NotFound && version.state == "captured" =>
            {
                let target = Path::new(&version.target);
                let adjacent = Path::new(&version.adjacent);
                if target.parent() != adjacent.parent() {
                    return Err(io::Error::other("recorded adjacent name is invalid"));
                }
                let adjacent_name = adjacent_name(&version)?.to_owned();
                let mut original = platform::edit_open(&parent.file, &adjacent_name)?;
                if platform::identity(&original)? != version.original_identity
                    || !matches_saved(
                        self,
                        &version,
                        &mut original,
                        "before",
                        version.before_modified,
                        budget,
                    )?
                {
                    return Err(io::Error::other(
                        "adjacent original changed; repair refused",
                    ));
                }
                platform::move_new(&parent.file, &original, &adjacent_name, &name)?;
                platform::sync_parent(&parent.file).map_err(|error| {
                    io::Error::other(format!(
                        "original restored at target but directory sync failed: {error}"
                    ))
                })?;
                self.record(&mut version, "not_published", None)
                    .map_err(|error| {
                        io::Error::other(format!(
                            "original restored at target but recovery checkpoint failed: {error}"
                        ))
                    })?;
                Ok("Original restored to absent target; no existing file was overwritten.".into())
            }
            Err(error)
                if error.kind() == io::ErrorKind::NotFound && version.state == "restoring" =>
            {
                let adjacent_name = adjacent_name(&version)?.to_owned();
                let mut previous = platform::edit_open(&parent.file, &adjacent_name)?;
                if version.published_identity != Some(platform::identity(&previous)?)
                    || !matches_saved(
                        self,
                        &version,
                        &mut previous,
                        "after",
                        version.after_modified,
                        budget,
                    )?
                {
                    return Err(io::Error::other(
                        "adjacent published result changed; repair refused",
                    ));
                }
                let stage_name = unique("staging");
                let mut stage = Stage {
                    parent: &parent.file,
                    file: platform::create(&parent.file, &stage_name)?,
                    name: stage_name,
                    published: false,
                };
                platform::apply_policy(&stage.file, &version.policy)?;
                let mut before = self.file(id, "before")?;
                if before.metadata()?.len() != version.before_bytes {
                    return Err(io::Error::other(
                        "retained original changed; repair refused",
                    ));
                }
                copy(&mut before, &mut stage.file, budget)?;
                stage
                    .file
                    .set_times(FileTimes::new().set_modified(version.before_modified))?;
                stage.file.sync_all()?;
                platform::move_new(&parent.file, &stage.file, &stage.name, &name)?;
                stage.published = true;
                platform::sync_parent(&parent.file).map_err(|error| {
                    io::Error::other(format!(
                        "original restored at target but directory sync failed: {error}"
                    ))
                })?;
                let identity = platform::identity(&stage.file)?;
                self.record(&mut version, "restored", Some(identity))
                    .map_err(|error| {
                        io::Error::other(format!(
                            "original restored at target but checkpoint failed: {error}"
                        ))
                    })?;
                platform::remove_owned(&parent.file, &previous, &adjacent_name).map_err(
                    |error| {
                        io::Error::other(format!(
                            "original restored at target but adjacent cleanup failed: {error}"
                        ))
                    },
                )?;
                Ok("Original restored to absent target; published result remains in private recovery.".into())
            }
            Err(error) => Err(error),
        }
    }
    pub fn inspect(
        &self,
        workspace: &Workspace,
        id: &str,
        budget: &Budget<'_>,
    ) -> io::Result<Inspection> {
        let version = self.get(id)?;
        require_workspace(workspace, &version)?;
        let (_, parent, name) = workspace
            .change_parent(&version.target, budget)
            .map_err(|_| io::ErrorKind::InvalidInput)?;
        let target = match platform::edit_open(&parent.file, &name) {
            Ok(mut file) => {
                if version.state == "capturing" {
                    return Ok(Inspection {
                        version,
                        target: "capture incomplete; target present",
                        adjacent: "not inspected",
                    });
                }
                let mut before = self.file(id, "before")?;
                let mut after = self.file(id, "after")?;
                if same(&mut file, &mut after, budget)? {
                    "published result"
                } else if same(&mut file, &mut before, budget)? {
                    "retained original"
                } else {
                    "conflict"
                }
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => "absent",
            Err(error) => return Err(error),
        };
        let adjacent = match Path::new(&version.adjacent).try_exists()? {
            true => "present; inspect before removing",
            false => "absent",
        };
        Ok(Inspection {
            version,
            target,
            adjacent,
        })
    }

    pub fn restore(
        &self,
        workspace: &Workspace,
        id: &str,
        budget: &Budget<'_>,
    ) -> io::Result<Restored> {
        self.restore_with(workspace, id, budget, || {}, || {})
    }
    fn restore_with(
        &self,
        workspace: &Workspace,
        id: &str,
        budget: &Budget<'_>,
        after_stash: impl FnOnce(),
        after_publish: impl FnOnce(),
    ) -> io::Result<Restored> {
        let mut version = self.get(id)?;
        require_workspace(workspace, &version)?;
        if !matches!(version.state.as_str(), "applied" | "restoring") {
            return Err(io::Error::other(
                "version is not confirmed applied; inspect it before manual recovery",
            ));
        }
        let (_, parent, name) = workspace
            .change_parent(&version.target, budget)
            .map_err(|_| io::Error::other("recorded target is unavailable"))?;
        let mut current = platform::edit_open(&parent.file, &name)?;
        platform::editable(&current)?;
        let current_identity = platform::identity(&current)?;
        let follows_restored_version = self.list()?.into_iter().any(|other| {
            other.id != version.id
                && other.state == "restored"
                && other.workspace == version.workspace
                && other.target == version.target
                && other.published_identity == Some(current_identity)
        });
        if version.published_identity != Some(current_identity) && !follows_restored_version
            || version.published_identity.is_none()
            || current.metadata()?.modified()? != version.after_modified
            || platform::capture_policy(&current)? != version.after_policy
        {
            return Err(io::Error::other(
                "target changed since this version was published; restoration refused",
            ));
        }
        let mut after = self.file(id, "after")?;
        if after.metadata()?.len() != version.after_bytes
            || !same(&mut current, &mut after, budget)?
        {
            return Err(io::Error::other(
                "target or retained result changed; restoration refused",
            ));
        }
        let mut before = self.file(id, "before")?;
        if before.metadata()?.len() != version.before_bytes {
            return Err(io::Error::other(
                "retained original has changed; restoration refused",
            ));
        }
        let stage_name = unique("staging");
        if Path::new(&version.adjacent).try_exists()? {
            return Err(io::Error::other(
                "adjacent transient remains; run recover repair before restoration",
            ));
        }
        let mut stage = Stage {
            parent: &parent.file,
            file: platform::create(&parent.file, &stage_name)?,
            name: stage_name,
            published: false,
        };
        platform::apply_policy(&stage.file, &version.policy)?;
        copy(&mut before, &mut stage.file, budget)?;
        stage
            .file
            .set_times(FileTimes::new().set_modified(version.before_modified))?;
        stage.file.sync_all()?;
        budget.check().map_err(|_| io::ErrorKind::Interrupted)?;
        let mut verify = self.file(id, "before")?;
        if !same(&mut stage.file, &mut verify, budget)? {
            return Err(io::Error::other(
                "retained original or staging file changed; restoration refused",
            ));
        }
        // This checkpoint is durable before the first filesystem effect.
        let published_identity = Some(current_identity);
        let adjacent = unique("staging");
        version.restore_identity = Some(platform::identity(&stage.file)?);
        version.adjacent = Path::new(&version.target)
            .with_file_name(&adjacent)
            .to_str()
            .ok_or(io::ErrorKind::InvalidData)?
            .into();
        self.record(&mut version, "restoring", published_identity)?;
        platform::move_new(&parent.file, &current, &name, &adjacent)?;
        after_stash();
        let published = (|| {
            #[cfg(not(windows))]
            {
                let mut moved = platform::edit_open(&parent.file, &adjacent)?;
                if platform::identity(&moved)? != version.published_identity.unwrap()
                    || !same(&mut moved, &mut after, budget)?
                {
                    return Err(io::Error::other("target changed during restoration"));
                }
            }
            #[cfg(windows)]
            if platform::identity(&current)? != version.published_identity.unwrap()
                || !same(&mut current, &mut after, budget)?
            {
                return Err(io::Error::other("target changed during restoration"));
            }
            budget.check().map_err(|_| io::ErrorKind::Interrupted)?;
            platform::move_new(&parent.file, &stage.file, &stage.name, &name)
        })();
        if let Err(error) = published {
            let repaired = platform::move_new(&parent.file, &current, &adjacent, &name).is_ok();
            return Err(io::Error::other(if repaired {
                format!("{error}; published result restored; selected version not installed")
            } else {
                format!(
                    "{error}; published result retained at adjacent {adjacent}; inspect recovery {id}"
                )
            }));
        }
        stage.published = true;
        after_publish();
        let mut warning = None;
        let checkpoint = platform::sync_parent(&parent.file)
            .and_then(|()| platform::identity(&stage.file))
            .and_then(|new_id| self.record(&mut version, "restored", Some(new_id)));
        if let Err(error) = checkpoint {
            warning = Some(format!(
                "original restored, but final recovery checkpoint failed: {error}; adjacent transient retained at {}",
                version.adjacent
            ));
        }
        if warning.is_none() {
            let cleanup = platform::remove_owned(&parent.file, &current, &adjacent)
                .and_then(|()| platform::sync_parent(&parent.file));
            if let Err(error) = cleanup {
                warning = Some(format!(
                    "original restored; adjacent cleanup or directory sync failed at {adjacent}: {error}"
                ));
            }
        }
        Ok(Restored { warning })
    }
}
#[cfg(test)]
#[path = "restore_tests.rs"]
mod tests;
fn adjacent_name(version: &Version) -> io::Result<&str> {
    let target = Path::new(&version.target);
    let adjacent = Path::new(&version.adjacent);
    if target.parent() != adjacent.parent() {
        return Err(io::Error::other("recorded adjacent name is invalid"));
    }
    adjacent
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| {
            name.starts_with(".jecode-staging-") || name.starts_with(".jecode-recovery-")
        })
        .ok_or_else(|| io::Error::other("recorded adjacent name is invalid"))
}
fn cleanup_adjacent(
    store: &RecoveryStore,
    version: &Version,
    parent: &File,
    expected: &str,
    expected_identity: (u64, u64),
    budget: &Budget<'_>,
) -> io::Result<&'static str> {
    let name = adjacent_name(version)?;
    let mut file = match platform::edit_open(parent, name) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Ok("No adjacent transient remains.");
        }
        Err(error) => return Err(error),
    };
    let modified = if expected == "before" {
        version.before_modified
    } else {
        version.after_modified
    };
    if platform::identity(&file)? != expected_identity
        || !matches_saved(store, version, &mut file, expected, modified, budget)?
    {
        return Err(io::Error::other(
            "adjacent transient changed; cleanup refused",
        ));
    }
    platform::remove_owned(parent, &file, name)?;
    platform::sync_parent(parent).map_err(|error| {
        io::Error::other(format!(
            "adjacent transient removed but directory sync failed: {error}"
        ))
    })?;
    Ok("Verified adjacent transient removed.")
}
fn matches_saved(
    store: &RecoveryStore,
    version: &Version,
    current: &mut File,
    suffix: &str,
    modified: SystemTime,
    budget: &Budget<'_>,
) -> io::Result<bool> {
    let policy = platform::capture_policy(current)?;
    let expected_policy = if suffix == "before" {
        &version.policy
    } else {
        &version.after_policy
    };
    Ok(current.metadata()?.modified()? == modified
        && policy == *expected_policy
        && matches_content(store, version, current, suffix, budget)?)
}
fn matches_content(
    store: &RecoveryStore,
    version: &Version,
    current: &mut File,
    suffix: &str,
    budget: &Budget<'_>,
) -> io::Result<bool> {
    let mut saved = store.file(&version.id, suffix)?;
    let expected = if suffix == "before" {
        version.before_bytes
    } else {
        version.after_bytes
    };
    Ok(saved.metadata()?.len() == expected && same(current, &mut saved, budget)?)
}
fn require_workspace(workspace: &Workspace, version: &Version) -> io::Result<()> {
    if workspace.path().to_str() != Some(&version.workspace)
        || !Path::new(&version.target).is_absolute()
    {
        return Err(io::Error::other(
            "recovery belongs to another selected workspace",
        ));
    }
    Ok(())
}
fn copy(source: &mut File, target: &mut File, budget: &Budget<'_>) -> io::Result<()> {
    use std::io::Seek;
    source.rewind()?;
    let mut buffer = [0; 16 * 1024];
    loop {
        budget.check().map_err(|_| io::ErrorKind::Interrupted)?;
        let n = source.read(&mut buffer)?;
        if n == 0 {
            return Ok(());
        }
        target.write_all(&buffer[..n])?;
    }
}
