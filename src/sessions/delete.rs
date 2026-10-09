use super::{Document, Store, journal, same_directory, valid_id};
use crate::{output, scratch};
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

#[derive(Default, Debug)]
pub(crate) struct DeleteReport {
    pub files: usize,
    pub directories: usize,
    pub legacy_references: usize,
}

pub(crate) struct Removal {
    bucket: PathBuf,
    ancillary: Vec<PathBuf>,
    durable: Vec<PathBuf>,
    output: output::Removal,
    scratch: Option<scratch::Removal>,
}

impl Store {
    /// Check owned parent paths before opening even the persistent lock file.
    /// This is also used when a fresh current handle needs its first lease.
    pub(super) fn deletion_scope(&self) -> Result<(), String> {
        inspect_parent(&self.bucket)?;
        inspect_parent(&self.output_directory())
    }

    /// Delete an inactive session while holding the same exclusive lease used
    /// by writers. The lock file remains in place for future lockers.
    pub(crate) fn delete(&self, id: &str) -> Result<DeleteReport, String> {
        if !valid_id(id) {
            return Err("Invalid session identifier".into());
        }
        self.deletion_scope()?;
        let _lease = self.acquire(id)?;
        let (document, _) = self.load(id)?;
        self.removal(&document)?.remove()
    }

    /// Called by the active handle while its existing lease and Live mutex are
    /// held. A fresh conversation need not have a checkpoint yet.
    pub(super) fn removal(&self, document: &Document) -> Result<Removal, String> {
        if !valid_id(&document.id) || !same_directory(&document.directory, &self.directory) {
            return Err("Session identity or working directory changed".into());
        }
        self.deletion_scope()?;
        let id = &document.id;
        let mut ancillary = Vec::new();
        let mut durable = Vec::new();
        let entries = fs::read_dir(&self.bucket)
            .map_err(|error| format!("Could not inspect session directory: {error}"))?;
        for entry in entries {
            let entry = entry.map_err(|error| error.to_string())?;
            let name = entry.file_name().to_string_lossy().into_owned();
            let path = entry.path();
            if name == format!("{id}.json")
                || name == format!("{id}.json.bak")
                || name == format!("{id}.jsonl")
            {
                inspect_file(&path)?;
                durable.push(path);
            } else if sidecar(&name, id) {
                inspect_file(&path)?;
                ancillary.push(path);
            }
        }
        let primary = self.bucket.join(format!("{id}.json"));
        let backup = self.bucket.join(format!("{id}.json.bak"));
        let journal_path = self.bucket.join(format!("{id}.jsonl"));
        let primary_state = legacy_state(&primary, id, &self.directory)?;
        let backup_state = legacy_state(&backup, id, &self.directory)?;
        // A corrupt backup may be a stale migration artifact, but its content
        // cannot establish ownership. Preserve it even when the journal loads.
        if backup_state == Legacy::Corrupt {
            return Err("Session backup is corrupt; files were left unchanged".into());
        }
        if primary_state == Legacy::Corrupt && backup_state != Legacy::Valid {
            return Err("Session snapshot is corrupt; files were left unchanged".into());
        }
        let journal_valid = if journal_path.exists() {
            matches!(
                journal::read(&journal_path, id, &self.directory)?,
                journal::Read::Saved(_)
            )
        } else {
            false
        };
        // A fresh active session has no durable file. An inactive caller has
        // already loaded and validated its record before reaching this point.
        if !durable.is_empty() {
            self.load(id)?;
        }
        let last = if journal_valid {
            &journal_path
        } else if primary_state == Legacy::Valid {
            &primary
        } else {
            &backup
        };
        durable.sort();
        if let Some(index) = durable.iter().position(|path| path == last) {
            let last = durable.remove(index);
            durable.push(last);
        }
        ancillary.sort();
        let output =
            output::prepare_removal(&self.output_directory(), &self.directory, id, document)?;
        let scratch = self.temporary_area(id)?.prepare_removal()?;
        Ok(Removal {
            bucket: self.bucket.clone(),
            ancillary,
            durable,
            output,
            scratch,
        })
    }
}

impl Removal {
    pub(crate) fn remove(self) -> Result<DeleteReport, String> {
        let mut report = DeleteReport::default();
        let output = self.output.remove().map_err(incomplete)?;
        report.files += output.files;
        report.directories += output.directories;
        report.legacy_references = output.legacy_references;
        if let Some(scratch) = self.scratch {
            let (files, directories) = scratch.remove().map_err(incomplete)?;
            report.files += files;
            report.directories += directories;
        }
        for path in self.ancillary.into_iter().chain(self.durable) {
            inspect_parent(&self.bucket).map_err(incomplete)?;
            inspect_file(&path).map_err(incomplete)?;
            fs::remove_file(&path).map_err(|error| {
                incomplete(format!("Could not remove {}: {error}", path.display()))
            })?;
            report.files += 1;
        }
        Ok(report)
    }
}

fn incomplete(message: impl AsRef<str>) -> String {
    format!(
        "Session deletion is incomplete; some owned files may have been removed: {}",
        message.as_ref()
    )
}

fn sidecar(name: &str, id: &str) -> bool {
    if name == format!("{id}.summary.json") {
        return true;
    }
    [
        format!("{id}.json.damaged-"),
        format!("{id}.jsonl.damaged-"),
        format!("{id}.summary.tmp-"),
        format!("{id}.tmp-"),
    ]
    .iter()
    .any(|prefix| name.strip_prefix(prefix).is_some_and(generated_id))
}

fn generated_id(value: &str) -> bool {
    let mut parts = value.split('-');
    (0..3).all(|_| {
        parts
            .next()
            .is_some_and(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
    }) && parts.next().is_none()
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Legacy {
    Missing,
    Valid,
    Corrupt,
}

fn legacy_state(path: &Path, id: &str, directory: &Path) -> Result<Legacy, String> {
    let Some(_) = file_metadata(path)? else {
        return Ok(Legacy::Missing);
    };
    let text = fs::read_to_string(path)
        .map_err(|error| format!("Could not read session snapshot: {error}"))?;
    let value = match crate::json::parse(&text) {
        Ok(value) => value,
        Err(_) => return Ok(Legacy::Corrupt),
    };
    if value.get("format").and_then(crate::json::Value::as_str) != Some("jecode.session")
        || value.get("format_version") != Some(&crate::json::Value::number(1))
    {
        return Err("Unsupported session snapshot format; files were left unchanged".into());
    }
    let document = match Document::parse(&value) {
        Ok(document) => document,
        Err(_) => return Ok(Legacy::Corrupt),
    };
    if document.id != id || !same_directory(&document.directory, directory) {
        return Err(
            "Session snapshot belongs to another identity or folder; files were left unchanged"
                .into(),
        );
    }
    Ok(Legacy::Valid)
}

fn inspect_parent(path: &Path) -> Result<(), String> {
    // The owner creates home/sessions/bucket and home/tmp/bucket. Reject a
    // replaced parent before following any descendant for removal.
    for parent in path
        .ancestors()
        .take(4)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
    {
        match fs::symlink_metadata(parent) {
            Ok(metadata) if linked(&metadata) || !metadata.is_dir() => {
                return Err(format!(
                    "Owned directory is not a regular directory: {}",
                    parent.display()
                ));
            }
            Ok(_) => {}
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            Err(error) => return Err(format!("Could not inspect owned directory: {error}")),
        }
    }
    Ok(())
}

fn inspect_file(path: &Path) -> Result<(), String> {
    if file_metadata(path)?.is_none() {
        return Err(format!(
            "Session file disappeared during deletion: {}",
            path.display()
        ));
    }
    Ok(())
}

fn file_metadata(path: &Path) -> Result<Option<fs::Metadata>, String> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if linked(&metadata) || !metadata.is_file() => Err(format!(
            "Owned session path is a link or special entry: {}",
            path.display()
        )),
        Ok(metadata) => Ok(Some(metadata)),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!("Could not inspect session file: {error}")),
    }
}

fn linked(metadata: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        metadata.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    {
        metadata.file_type().is_symlink()
    }
}

#[cfg(test)]
mod tests;
