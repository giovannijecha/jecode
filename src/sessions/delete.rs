use super::{Document, Store, same_directory, valid_id};
use crate::{output, scratch};
use std::collections::BTreeSet;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

#[derive(Default, Debug)]
pub(crate) struct DeleteReport {
    pub files: usize,
    pub directories: usize,
}

pub(crate) struct Removal {
    bucket: PathBuf,
    ancillary: Vec<PathBuf>,
    durable: Option<PathBuf>,
    output: output::Removal,
    scratch: Option<scratch::Removal>,
    /// Attachments the deleted conversation sent; no grace period applies.
    released: BTreeSet<String>,
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
        let mut durable = None;
        let entries = fs::read_dir(&self.bucket)
            .map_err(|error| format!("Could not inspect session directory: {error}"))?;
        for entry in entries {
            let entry = entry.map_err(|error| error.to_string())?;
            let name = entry.file_name().to_string_lossy().into_owned();
            let path = entry.path();
            if name == format!("{id}.jsonl") {
                inspect_file(&path)?;
                durable = Some(path);
            } else if name == format!("{id}.json") || name == format!("{id}.json.bak") {
                inspect_file(&path)?;
                inspect_original(&path, document)?;
                ancillary.push(path);
            } else if sidecar(&name, id) {
                inspect_file(&path)?;
                ancillary.push(path);
            }
        }
        // A fresh active session has no durable file. An inactive caller has
        // already loaded and validated its record before reaching this point.
        if durable.is_some() {
            self.load(id)?;
        }
        ancillary.sort();
        let output =
            output::prepare_removal(&self.output_directory(), &self.directory, id, document)?;
        let scratch = self.temporary_area(id)?.prepare_removal()?;
        // Draft attachments are left to the regular grace period: a fresh
        // conversation may inherit the draft before its first save.
        let released = document
            .messages
            .iter()
            .flat_map(|message| crate::attachments::references(message.encode().as_bytes()))
            .collect();
        Ok(Removal {
            bucket: self.bucket.clone(),
            released,
            ancillary,
            durable,
            output,
            scratch,
        })
    }
}

impl Removal {
    pub(crate) fn remove(self) -> Result<DeleteReport, String> {
        self.remove_with_live(&BTreeSet::new())
    }

    pub(crate) fn remove_with_live(
        self,
        live_attachments: &BTreeSet<String>,
    ) -> Result<DeleteReport, String> {
        let mut report = DeleteReport::default();
        let output = self.output.remove().map_err(incomplete)?;
        report.files += output.files;
        report.directories += output.directories;

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
        // Collection is best effort: the conversation itself is already gone.
        report.files += crate::attachments::Pool::new(self.bucket.join("attachments"))
            .collect(&self.bucket, live_attachments, &self.released)
            .unwrap_or(0);
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

/// Migration kept these originals beside the current journal. Deletion must
/// release their conversation data without restoring the old resume path.
fn inspect_original(path: &Path, document: &Document) -> Result<(), String> {
    use crate::json::{self, Value};
    let value = fs::read_to_string(path)
        .map_err(|error| error.to_string())
        .and_then(|text| json::parse(&text))
        .map_err(|error| format!("Could not verify original session copy: {error}"))?;
    let directory = value
        .get("working_directory")
        .and_then(Value::as_str)
        .map(Path::new);
    if value.get("format").and_then(Value::as_str) != Some("jecode.session")
        || value.get("format_version") != Some(&Value::number(1))
        || value.get("id").and_then(Value::as_str) != Some(document.id.as_str())
        || !directory.is_some_and(|path| same_directory(path, &document.directory))
    {
        return Err("Original session copy belongs to another identity or folder; files were left unchanged".into());
    }
    Ok(())
}

fn generated_id(value: &str) -> bool {
    let mut parts = value.split('-');
    (0..3).all(|_| {
        parts
            .next()
            .is_some_and(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
    }) && parts.next().is_none()
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
            Ok(metadata) if crate::disk::linked(&metadata) || !metadata.is_dir() => {
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
    match fs::symlink_metadata(path) {
        Ok(metadata) if crate::disk::linked(&metadata) || !metadata.is_file() => Err(format!(
            "Owned session path is a link or special entry: {}",
            path.display()
        )),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == ErrorKind::NotFound => Err(format!(
            "Session file disappeared during deletion: {}",
            path.display()
        )),
        Err(error) => Err(format!("Could not inspect session file: {error}")),
    }
}

#[cfg(test)]
mod tests;
