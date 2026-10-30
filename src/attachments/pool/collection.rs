//! Collect assets against the project's own sessions and retained collaborator journals.

use super::{Pool, references};
use crate::disk;
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

fn collect_references(sessions: &Path, referenced: &mut BTreeSet<String>) -> Result<(), String> {
    for entry in directory(sessions, false)?.expect("required directory") {
        let entry = entry.map_err(inspect_error)?;
        let name = entry.file_name();
        if name.to_string_lossy() == "collaboration" {
            collect_collaboration(&entry.path(), referenced)?;
        } else if !name.to_string_lossy().ends_with(".lock") {
            read_file(&entry.path(), referenced)?;
        }
    }
    Ok(())
}

fn collect_collaboration(
    collaboration: &Path,
    referenced: &mut BTreeSet<String>,
) -> Result<(), String> {
    let Some(parents) = directory(collaboration, true)? else {
        return Ok(());
    };
    for parent in parents {
        let parent = parent.map_err(inspect_error)?;
        let name = parent.file_name();
        if !crate::sessions::valid_id(&name.to_string_lossy()) {
            continue;
        }
        let Some(entries) = directory(&parent.path(), true)? else {
            continue;
        };
        for entry in entries {
            let entry = entry.map_err(inspect_error)?;
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name == "child-home" {
                collect_child_home(&entry.path(), referenced)?;
            } else if name.starts_with("parent-") && name.ends_with(".json") {
                read_file(&entry.path(), referenced)?;
            }
        }
    }
    Ok(())
}

fn collect_child_home(home: &Path, referenced: &mut BTreeSet<String>) -> Result<(), String> {
    let Some(_) = directory(home, true)? else {
        return Ok(());
    };
    let Some(buckets) = directory(&home.join("sessions"), true)? else {
        return Ok(());
    };
    for bucket in buckets {
        let bucket = bucket.map_err(inspect_error)?;
        let name = bucket.file_name();
        let name = name.to_string_lossy();
        if name.len() != 32 || !name.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            continue;
        }
        let Some(files) = directory(&bucket.path(), true)? else {
            continue;
        };
        for file in files {
            let file = file.map_err(inspect_error)?;
            if !file.file_name().to_string_lossy().ends_with(".lock") {
                read_file(&file.path(), referenced)?;
            }
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
    if metadata.is_file() {
        referenced.extend(references(&fs::read(path).map_err(inspect_error)?));
    }
    Ok(())
}

fn inspect_error(error: std::io::Error) -> String {
    format!("Cannot inspect sessions: {error}")
}

#[cfg(test)]
mod tests;
