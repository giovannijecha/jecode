//! Collect assets against the project's own sessions and retained collaborator journals.

use super::{Pool, references};
use crate::disk;
use crate::sessions::layout;
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, ReadDir};
use std::io::ErrorKind;
use std::path::Path;
use std::time::{Duration, SystemTime};

/// A recent unreferenced asset may belong to another process's unsaved draft.
const GRACE: Duration = Duration::from_secs(600);

impl Pool {
    /// Removes assets no session file, live state or recent import refers to.
    /// `released` assets skip the grace period: their owner was deleted.
    pub fn collect(
        &self,
        sessions: &Path,
        live: &BTreeSet<String>,
        released: &BTreeSet<String>,
    ) -> Result<usize, String> {
        let entries = match fs::read_dir(&self.directory) {
            Ok(entries) => entries,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(0),
            Err(error) => return Err(format!("Cannot inspect attachments: {error}")),
        };
        // Finish reading every journal before removing anything. A missing or
        // unreadable child journal must not make its assets look unreferenced.
        let mut referenced = live.clone();
        collect_references(sessions, &mut referenced)?;

        let now = SystemTime::now();
        // The youngest entry of an asset decides its age.
        let mut ages = BTreeMap::<String, Duration>::new();
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let id = name.split('.').next().unwrap_or_default().to_owned();
            if !super::super::valid_id(&id) {
                continue;
            }
            let age = entry
                .metadata()
                .and_then(|metadata| metadata.modified())
                .ok()
                .and_then(|modified| now.duration_since(modified).ok())
                .unwrap_or_default();
            ages.entry(id)
                .and_modify(|current| *current = (*current).min(age))
                .or_insert(age);
        }
        let mut removed = 0;
        for (id, age) in ages {
            if !referenced.contains(&id) && (released.contains(&id) || age >= GRACE) {
                self.remove(&id);
                removed += 1;
            }
        }
        Ok(removed)
    }
}

/// Reads every record in a project's session folder that can name an asset:
/// each session's journal and sidecars, its retained collaborator records and
/// its collaborators' own sessions. Files of the earlier flat layout are read
/// too, so their assets are kept although those sessions cannot be opened.
fn collect_references(sessions: &Path, referenced: &mut BTreeSet<String>) -> Result<(), String> {
    collect_bucket(sessions, false, referenced)
}

fn collect_bucket(
    bucket: &Path,
    optional: bool,
    referenced: &mut BTreeSet<String>,
) -> Result<(), String> {
    let Some(entries) = directory(bucket, optional)? else {
        return Ok(());
    };
    for entry in entries {
        let entry = entry.map_err(inspect_error)?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let path = entry.path();
        if crate::sessions::valid_id(&name)
            && disk::ordinary_directory(&fs::symlink_metadata(&path).map_err(inspect_error)?)
        {
            collect_session(&path, referenced)?;
        } else if name == layout::COLLABORATION {
            // The earlier layout kept each parent's collaborators here.
            let Some(parents) = directory(&path, true)? else {
                continue;
            };
            for parent in parents {
                let parent = parent.map_err(inspect_error)?;
                if crate::sessions::valid_id(&parent.file_name().to_string_lossy()) {
                    collect_collaboration(&parent.path(), referenced)?;
                }
            }
        } else if !name.ends_with(".lock") {
            // Journals and sidecars of the earlier layout. Directories such
            // as the attachment pool hold no records.
            read_file(&path, referenced)?;
        }
    }
    Ok(())
}

/// One session's own records; its saved outputs and temporary files are not
/// records, and its lock holds nothing.
fn collect_session(session: &Path, referenced: &mut BTreeSet<String>) -> Result<(), String> {
    for entry in directory(session, false)?.expect("required directory") {
        let entry = entry.map_err(inspect_error)?;
        let name = entry.file_name().to_string_lossy().into_owned();
        match name.as_str() {
            layout::LOCK | layout::OUTPUTS | layout::TEMPORARY => {}
            layout::COLLABORATION => collect_collaboration(&entry.path(), referenced)?,
            _ => read_file(&entry.path(), referenced)?,
        }
    }
    Ok(())
}

/// Retained parent records and the collaborators' own sessions; isolated
/// workspaces hold project files, not records.
fn collect_collaboration(
    collaboration: &Path,
    referenced: &mut BTreeSet<String>,
) -> Result<(), String> {
    let Some(entries) = directory(collaboration, true)? else {
        return Ok(());
    };
    for entry in entries {
        let entry = entry.map_err(inspect_error)?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name == layout::CHILD_HOME {
            collect_child_home(&entry.path(), referenced)?;
        } else if name.starts_with("parent-") && name.ends_with(".json") {
            read_file(&entry.path(), referenced)?;
        }
    }
    Ok(())
}

fn collect_child_home(home: &Path, referenced: &mut BTreeSet<String>) -> Result<(), String> {
    let Some(_) = directory(home, true)? else {
        return Ok(());
    };
    let Some(buckets) = directory(&home.join(layout::SESSIONS), true)? else {
        return Ok(());
    };
    for bucket in buckets {
        let bucket = bucket.map_err(inspect_error)?;
        if layout::bucket_name(&bucket.file_name().to_string_lossy()) {
            collect_bucket(&bucket.path(), true, referenced)?;
        }
    }
    Ok(())
}

fn directory(path: &Path, optional: bool) -> Result<Option<ReadDir>, String> {
    let metadata = match disk::metadata_if_present(path).map_err(inspect_error)? {
        Some(metadata) => metadata,
        None if optional => return Ok(None),
        None => {
            return Err(format!(
                "Cannot inspect sessions: {} is missing",
                path.display()
            ));
        }
    };
    if !disk::ordinary_directory(&metadata) {
        return Err(format!(
            "Cannot inspect sessions: {} is not an ordinary directory",
            path.display()
        ));
    }
    fs::read_dir(path).map(Some).map_err(inspect_error)
}

fn read_file(path: &Path, referenced: &mut BTreeSet<String>) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path).map_err(inspect_error)?;
    if disk::linked(&metadata) {
        return Err(format!(
            "Cannot inspect sessions: {} is a link or reparse point",
            path.display()
        ));
    }
    if metadata.is_dir() {
        if path.extension().and_then(|extension| extension.to_str()) == Some("jsonl") {
            return Err(format!(
                "Cannot inspect sessions: {} is not a file",
                path.display()
            ));
        }
        return Ok(());
    }
    if !metadata.is_file() {
        return Err(format!(
            "Cannot inspect sessions: {} is not a regular file",
            path.display()
        ));
    }
    referenced.extend(references(&fs::read(path).map_err(inspect_error)?));
    Ok(())
}

fn inspect_error(error: std::io::Error) -> String {
    format!("Cannot inspect sessions: {error}")
}

#[cfg(test)]
mod tests;
