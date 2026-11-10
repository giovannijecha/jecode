use super::{Document, Store, layout, same_directory, valid_id};
use crate::{json::Value, output, scratch};
use std::collections::BTreeSet;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

#[derive(Default, Debug)]
pub(crate) struct DeleteReport {
    pub files: usize,
    pub directories: usize,
    pub retained_collaboration: Option<PathBuf>,
    /// Entries of the session folder that deletion does not own, left in place.
    pub kept: Vec<PathBuf>,
}

struct RetainedCollaboration {
    directory: PathBuf,
    manifest: PathBuf,
    bytes: Vec<u8>,
}

pub(crate) struct Removal {
    home: PathBuf,
    session: PathBuf,
    attachment_bucket: PathBuf,
    ancillary: Vec<PathBuf>,
    durable: Option<PathBuf>,
    output: output::Removal,
    scratch: Option<scratch::Removal>,
    collaboration: Option<RetainedCollaboration>,
    kept: Vec<PathBuf>,
    /// Attachments the deleted conversation sent; no grace period applies.
    released: BTreeSet<String>,
}

impl Store {
    /// Check the owned folders from the home down to session `id` before
    /// opening even the persistent lock file. This is also used when a fresh
    /// current handle needs its first lease.
    pub(super) fn deletion_scope(&self, id: &str) -> Result<(), String> {
        inspect_owned(self.home(), &self.session_directory(id))
    }

    /// Delete an inactive session while holding the same exclusive lease used
    /// by writers. The lock file remains in place for future lockers.
    pub(crate) fn delete(&self, id: &str) -> Result<DeleteReport, String> {
        if !valid_id(id) {
            return Err("Invalid session identifier".into());
        }
        self.deletion_scope(id)?;
        // A missing session gets no folder or lock; the load under the lease
        // below remains the authoritative check.
        if !self.journal_path(id).exists() {
            self.load(id)?;
        }
        let _lease = self.acquire(id)?;
        let (document, _) = self.load(id)?;
        self.removal(&document)?.remove()
    }

    /// Called by the active handle while its existing lease and Live mutex are
    /// held. A fresh conversation need not have a checkpoint yet. Everything
    /// to remove is inspected before anything is removed.
    pub(super) fn removal(&self, document: &Document) -> Result<Removal, String> {
        if !valid_id(&document.id) || !same_directory(&document.directory, &self.directory) {
            return Err("Session identity or working directory changed".into());
        }
        let id = &document.id;
        self.deletion_scope(id)?;
        let session = self.session_directory(id);
        let entries = match fs::read_dir(&session) {
            Ok(entries) => entries
                .collect::<Result<Vec<_>, _>>()
                .map_err(|error| format!("Could not inspect session directory: {error}"))?,
            Err(error) if error.kind() == ErrorKind::NotFound => Vec::new(),
            Err(error) => return Err(format!("Could not inspect session directory: {error}")),
        };
        let mut ancillary = Vec::new();
        let mut durable = None;
        let mut kept = Vec::new();
        for entry in entries {
            let name = entry.file_name().to_string_lossy().into_owned();
            let path = entry.path();
            match name.as_str() {
                layout::JOURNAL => {
                    inspect_file(&path)?;
                    durable = Some(path);
                }
                // The lock stays for other lockers; the owned folders are
                // inspected by their own removals below.
                layout::LOCK | layout::OUTPUTS | layout::TEMPORARY | layout::COLLABORATION => {}
                _ if sidecar(&name) => {
                    inspect_file(&path)?;
                    ancillary.push(path);
                }
                _ => kept.push(path),
            }
        }
        // A fresh active session has no durable file. An inactive caller has
        // already loaded and validated its record before reaching this point.
        if durable.is_some() {
            self.load(id)?;
        }
        ancillary.sort();
        kept.sort();
        let output = output::prepare_removal(&self.bucket, &self.directory, id, document)?;
        let scratch = self.temporary_area(id)?.prepare_removal()?;
        let collaboration = RetainedCollaboration::prepare(&self.collaboration(id), document)?;
        // Draft attachments are left to the regular grace period: a fresh
        // conversation may inherit the draft before its first save.
        let released = document
            .messages
            .iter()
            .flat_map(|message| crate::attachments::references(message.encode().as_bytes()))
            .collect();
        Ok(Removal {
            home: self.home().to_path_buf(),
            session,
            attachment_bucket: self.attachment_bucket().to_path_buf(),
            released,
            ancillary,
            durable,
            output,
            scratch,
            collaboration,
            kept,
        })
    }
}

impl Removal {
    pub(crate) fn remove(self) -> Result<DeleteReport, String> {
        self.remove_with_live(&BTreeSet::new())
    }

    /// Retains collaborator records first, then removes outputs, the
    /// temporary area and sidecars; the journal goes last, so a failure
    /// leaves the conversation available for a later attempt.
    pub(crate) fn remove_with_live(
        self,
        live_attachments: &BTreeSet<String>,
    ) -> Result<DeleteReport, String> {
        let mut report = DeleteReport::default();
        if let Some(collaboration) = &self.collaboration {
            inspect_owned(&self.home, &collaboration.directory)?;
            collaboration.save()?;
            report.retained_collaboration = Some(collaboration.directory.clone());
        }
        let output = self.output.remove().map_err(incomplete)?;
        report.files += output.files;
        report.directories += output.directories;

        if let Some(scratch) = self.scratch {
            let (files, directories) = scratch.remove().map_err(incomplete)?;
            report.files += files;
            report.directories += directories;
        }
        for path in self.ancillary.into_iter().chain(self.durable) {
            inspect_owned(&self.home, &self.session).map_err(incomplete)?;
            inspect_file(&path).map_err(incomplete)?;
            fs::remove_file(&path).map_err(|error| {
                incomplete(format!("Could not remove {}: {error}", path.display()))
            })?;
            report.files += 1;
        }
        report.kept = self.kept;
        // Collection is best effort: the conversation itself is already gone.
        report.files +=
            crate::attachments::Pool::new(self.attachment_bucket.join(layout::ATTACHMENTS))
                .collect(&self.attachment_bucket, live_attachments, &self.released)
                .unwrap_or(0);
        Ok(report)
    }
}

impl RetainedCollaboration {
    fn prepare(directory: &Path, document: &Document) -> Result<Option<Self>, String> {
        let directory = directory.to_path_buf();
        let Some(metadata) = crate::disk::metadata_if_present(&directory)
            .map_err(|error| format!("Could not inspect collaborator files: {error}"))?
        else {
            return Ok(None);
        };
        if !crate::disk::ordinary_directory(&metadata) {
            return Err("Collaborator storage is not an owned directory".into());
        }
        if !contains_file(&directory)? {
            return Ok(None);
        }
        let last = document
            .events
            .iter()
            .rev()
            .find(|event| event.get("type").and_then(Value::as_str) == Some("collaboration"));
        let sequence = last
            .and_then(|event| event.get("sequence"))
            .and_then(Value::as_usize)
            .unwrap_or(0);
        let records = last
            .and_then(|event| event.get("records"))
            .and_then(Value::as_array)
            .unwrap_or(&[])
            .iter()
            .map(|record| {
                let field = |name| record.get(name).cloned().unwrap_or(Value::Null);
                Value::object([
                    ("id", field("id")),
                    ("number", field("number")),
                    ("title", field("title")),
                    ("state", field("state")),
                    ("workspace", field("workspace")),
                    ("workspace_info", field("workspace_info")),
                    ("directory", field("directory")),
                    ("workspace_closed", field("workspace_closed")),
                    ("sent_version", field("sent_version")),
                    ("received_version", field("received_version")),
                    ("report", field("report")),
                    ("reviewed", field("reviewed")),
                    ("error", field("error")),
                ])
            })
            .collect();
        let manifest = directory.join(format!("parent-{sequence}.json"));
        let bytes = Value::object([
            ("format", Value::string("jecode.collaboration.retained")),
            ("format_version", Value::number(1)),
            ("parent_session_id", Value::string(&document.id)),
            (
                "working_directory",
                Value::string(document.directory.to_string_lossy()),
            ),
            ("receipt_sequence", Value::number(sequence)),
            ("records", Value::Array(records)),
        ])
        .encode()
        .into_bytes();
        Ok(Some(Self {
            directory,
            manifest,
            bytes,
        }))
    }

    fn save(&self) -> Result<(), String> {
        match crate::disk::metadata_if_present(&self.manifest)
            .map_err(|error| format!("Could not inspect retained collaborator record: {error}"))?
        {
            Some(metadata) => {
                if crate::disk::linked(&metadata) || !metadata.is_file() {
                    return Err("Retained collaborator record is not an ordinary file".into());
                }
                if fs::read(&self.manifest).map_err(|error| error.to_string())? != self.bytes {
                    return Err(
                        "Retained collaborator record changed; session was not deleted".into(),
                    );
                }
            }
            None => crate::disk::atomic_write(&self.manifest, &self.bytes)
                .map_err(|error| format!("Could not retain collaborator record: {error}"))?,
        }
        Ok(())
    }
}

fn contains_file(directory: &Path) -> Result<bool, String> {
    let mut pending = vec![directory.to_path_buf()];
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(&directory)
            .map_err(|error| format!("Could not inspect collaborator files: {error}"))?
        {
            let entry = entry.map_err(|error| error.to_string())?;
            let metadata = fs::symlink_metadata(entry.path())
                .map_err(|error| format!("Could not inspect collaborator file: {error}"))?;
            if crate::disk::ordinary_directory(&metadata) {
                pending.push(entry.path());
            } else {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

fn incomplete(message: impl AsRef<str>) -> String {
    format!(
        "Session deletion is incomplete; some owned files may have been removed: {}",
        message.as_ref()
    )
}

/// The summary cache, its staging files and set-aside damaged journal tails,
/// named as the summary and journal writers name them.
fn sidecar(name: &str) -> bool {
    if name == layout::SUMMARY {
        return true;
    }
    [
        Path::new(layout::JOURNAL).with_extension("jsonl.damaged-"),
        Path::new(layout::SUMMARY).with_extension("tmp-"),
    ]
    .iter()
    .any(|prefix| {
        name.strip_prefix(prefix.to_string_lossy().as_ref())
            .is_some_and(generated_id)
    })
}

fn generated_id(value: &str) -> bool {
    let mut parts = value.split('-');
    (0..3).all(|_| {
        parts
            .next()
            .is_some_and(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
    }) && parts.next().is_none()
}

/// Every folder from `home` down to `path` that exists is an ordinary
/// directory. The owner creates them, so a replaced one is rejected before
/// any descendant is followed for removal.
fn inspect_owned(home: &Path, path: &Path) -> Result<(), String> {
    let below = path
        .strip_prefix(home)
        .map_err(|_| "Owned session path is outside its home")?;
    let mut current = home.to_path_buf();
    for part in std::iter::once(None).chain(below.components().map(Some)) {
        if let Some(part) = part {
            current.push(part);
        }
        match fs::symlink_metadata(&current) {
            Ok(metadata) if crate::disk::linked(&metadata) || !metadata.is_dir() => {
                return Err(format!(
                    "Owned directory is not a regular directory: {}",
                    current.display()
                ));
            }
            Ok(_) => {}
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(()),
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
