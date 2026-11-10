use super::{Document, canonical, layout, same_directory, valid_id};
use std::collections::BTreeSet;
use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

#[derive(Clone)]
pub struct Store {
    home: PathBuf,
    pub(super) directory: PathBuf,
    pub(super) bucket: PathBuf,
    attachment_bucket: PathBuf,
}

pub struct Summary {
    pub id: String,
    pub title: String,
    pub updated: u64,
    pub backend: String,
    pub model: String,
}

impl Summary {
    pub fn description(&self) -> String {
        format!("{} · {}", self.age(), self.origin())
    }
    pub fn age(&self) -> String {
        match self.elapsed() {
            now if now == "just now" => now,
            elapsed => format!("{elapsed} ago"),
        }
    }
    /// The compact age: `just now`, `5m`, `2h`, `3d`.
    pub fn elapsed(&self) -> String {
        let age = super::now().saturating_sub(self.updated) / 1000;
        match age {
            0..60 => "just now".into(),
            60..3600 => format!("{}m", age / 60),
            3600..86400 => format!("{}h", age / 3600),
            _ => format!("{}d", age / 86400),
        }
    }
    /// Backend, model and ID.
    pub fn origin(&self) -> String {
        format!("{} / {} · {}", self.backend, self.model, self.id)
    }
}

#[derive(Default)]
pub struct Listing {
    pub sessions: Vec<Summary>,
    pub warnings: Vec<String>,
}

pub(super) struct Lease {
    // The lock covers the append journal.
    _lock: File,
    store: Store,
    id: String,
    journal: Mutex<Option<super::journal::Position>>,
}

impl Drop for Lease {
    fn drop(&mut self) {
        // A concurrent Unix fork can retain a duplicate until exec closes it.
        // Unlock explicitly so releasing this lease also releases that lock.
        let _ = self._lock.unlock();
    }
}

impl Store {
    /// The folder session `id` owns: its journal, lock, summary, saved
    /// outputs, temporary area and collaborator storage.
    pub(crate) fn session_directory(&self, id: &str) -> PathBuf {
        self.bucket.join(id)
    }

    /// Saved tool outputs of session `id`, which also resolve the outputs of
    /// the collaborators it starts.
    pub fn outputs(
        &self,
        id: &str,
        redactor: crate::redact::Redactor,
    ) -> Result<crate::output::Store, String> {
        Ok(crate::output::Store::for_session(
            self.bucket.clone(),
            self.directory.clone(),
            id,
            redactor,
        )?
        .with_collaborators(self.collaborator_home(id)))
    }

    /// Where session `id` keeps its collaborators' workspaces and records.
    pub fn collaboration(&self, id: &str) -> PathBuf {
        self.session_directory(id).join(layout::COLLABORATION)
    }

    /// Where the collaborators of session `id` keep their own sessions.
    pub fn collaborator_home(&self, id: &str) -> PathBuf {
        self.collaboration(id).join(layout::CHILD_HOME)
    }

    /// Session-owned copies of attached files, shared by this project's sessions.
    pub fn attachments(&self) -> crate::attachments::Pool {
        crate::attachments::Pool::new(self.attachment_bucket.join(layout::ATTACHMENTS))
    }

    /// The session tree that owns this store's shared attachment pool.
    pub(crate) fn attachment_bucket(&self) -> &Path {
        &self.attachment_bucket
    }

    pub(crate) fn share_attachments(&mut self, parent: &Store) {
        self.attachment_bucket = parent.attachment_bucket.clone();
    }

    /// Where this project's session folders live.
    pub fn bucket(&self) -> &Path {
        &self.bucket
    }

    /// The Jecode home, or a collaborator's own home, holding these sessions.
    pub fn home(&self) -> &Path {
        &self.home
    }

    /// The canonical project directory these sessions belong to.
    pub fn directory(&self) -> &Path {
        &self.directory
    }

    pub fn temporary_area(&self, id: &str) -> Result<crate::scratch::Area, String> {
        crate::scratch::Area::new(
            &self.home,
            self.session_directory(id).join(layout::TEMPORARY),
            self.directory.clone(),
            id,
        )
    }

    /// Where a session journal belongs, for measurements of copied journals.
    #[cfg(test)]
    pub fn fixture_journal(&self, id: &str) -> PathBuf {
        self.journal_path(id)
    }

    #[cfg(test)]
    pub fn fixture_load(&self, id: &str) -> Result<Document, String> {
        self.read(id)
    }

    /// A saved document, read without opening its session or changing files.
    pub fn read(&self, id: &str) -> Result<Document, String> {
        self.load(id).map(|(document, _)| document)
    }

    pub fn new(home: PathBuf, directory: &Path) -> Result<Self, String> {
        if !home.is_absolute() {
            return Err("Session configuration directory must be absolute".into());
        }
        let directory = canonical(directory)?;
        let bucket = layout::bucket(&home, &directory);
        Ok(Self {
            home,
            directory,
            attachment_bucket: bucket.clone(),
            bucket,
        })
    }

    /// Saved conversations. A folder without a journal, such as a fresh
    /// session's temporary area or a deleted one's retained collaborator
    /// files, is not a conversation. Sessions of the earlier flat layout are
    /// reported and left unchanged.
    pub fn list(&self) -> Result<Listing, String> {
        let entries = match fs::read_dir(&self.bucket) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Listing::default());
            }
            Err(error) => return Err(format!("Could not list sessions: {error}")),
        };
        let mut ids = BTreeSet::new();
        let mut legacy = BTreeSet::new();
        let mut warnings = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|error| format!("Could not list sessions: {error}"))?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if valid_id(&name) {
                let path = entry.path();
                match crate::disk::metadata_if_present(&path) {
                    Ok(Some(metadata)) if crate::disk::ordinary_directory(&metadata) => {
                        match crate::disk::metadata_if_present(&path.join(layout::JOURNAL)) {
                            Ok(Some(_)) => {
                                ids.insert(name);
                            }
                            Ok(None) => {}
                            Err(error) => warnings.push(format!(
                                "Session {name}: Could not inspect session journal: {error}"
                            )),
                        }
                    }
                    Ok(Some(metadata)) if crate::disk::linked(&metadata) => warnings.push(format!(
                        "Session {name}: session folder is a link; it was left unchanged"
                    )),
                    Ok(_) => {}
                    Err(error) => warnings.push(format!("Session {name}: {error}")),
                }
            } else if let Some(id) = legacy_id(&name) {
                legacy.insert(id.to_string());
            }
        }
        let mut listing = Listing::default();
        for id in legacy.difference(&ids) {
            listing
                .warnings
                .push(format!("Session {id}: {}", super::LEGACY));
        }
        listing.warnings.extend(warnings);
        for id in ids {
            let stamp = super::summary::stamp(self, &id);
            if let Some(summary) = stamp
                .as_ref()
                .and_then(|stamp| super::summary::load(self, &id, stamp))
            {
                listing.sessions.push(summary);
                continue;
            }
            match self.load(&id) {
                Ok((document, _)) => {
                    if let Some(stamp) = &stamp {
                        // The cache is optional; opening still validates the journal itself.
                        let _ = super::summary::save(self, &document, &document.title(), stamp);
                    }
                    listing.sessions.push(Summary {
                        title: document.title(),
                        updated: document.updated,
                        model: document.model_label().into(),
                        backend: document.backend,
                        id: document.id,
                    });
                }
                Err(error) => listing.warnings.push(format!("Session {id}: {error}")),
            }
        }
        listing
            .sessions
            .sort_by(|a, b| b.updated.cmp(&a.updated).then_with(|| b.id.cmp(&a.id)));
        Ok(listing)
    }

    /// Refuses an identifier that only the earlier flat layout saved: its
    /// files stay as they are, and no folder is created beside them.
    fn reject_legacy(&self, id: &str) -> Result<(), String> {
        let present = |path: &Path| {
            crate::disk::metadata_if_present(path)
                .ok()
                .flatten()
                .is_some()
        };
        let legacy = [format!("{id}.jsonl"), format!("{id}.json")]
            .iter()
            .any(|name| present(&self.bucket.join(name)));
        if legacy && !present(&self.journal_path(id)) {
            return Err(super::LEGACY.into());
        }
        Ok(())
    }

    pub(super) fn acquire(&self, id: &str) -> Result<Lease, String> {
        if !valid_id(id) {
            return Err("Invalid session identifier".into());
        }
        self.reject_legacy(id)?;
        let directory = self.session_directory(id);
        crate::disk::private_directory(&self.bucket)
            .and_then(|()| crate::disk::ensure_private_directory(&directory))
            .map_err(|error| format!("Could not create session directory: {error}"))?;
        let path = directory.join(layout::LOCK);
        reject_special(&path)?;
        // The lock file is never removed, even by deletion: another process
        // may hold or wait on it, and a new file would not exclude that one.
        let lock = crate::disk::private_options()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)
            .map_err(|error| format!("Could not open session lock: {error}"))?;
        lock.try_lock()
            .map_err(|error| format!("Session is already open or cannot be locked: {error}"))?;
        Ok(Lease {
            _lock: lock,
            store: self.clone(),
            id: id.into(),
            journal: Mutex::new(None),
        })
    }

    pub(super) fn create(&self, id: &str) -> Result<Lease, String> {
        let lease = self.acquire(id)?;
        if self.journal_path(id).exists() {
            return Err("Session identifier already exists; saved data was left unchanged".into());
        }
        Ok(lease)
    }

    /// The saved document, and whether a damaged journal tail was set aside.
    pub(super) fn load(&self, id: &str) -> Result<(Document, bool), String> {
        if !valid_id(id) {
            return Err("Invalid session identifier".into());
        }
        self.reject_legacy(id)?;
        if let Some(metadata) = crate::disk::metadata_if_present(&self.session_directory(id))
            .map_err(|error| format!("Could not inspect session directory: {error}"))?
            && !crate::disk::ordinary_directory(&metadata)
        {
            return Err(
                "Session folder is a link or not a directory; it was left unchanged".into(),
            );
        }
        let journal = self.journal_path(id);
        if !journal.exists() {
            return Err("Session not found in this working directory".into());
        }
        match super::journal::read(&journal, id, &self.directory)? {
            super::journal::Read::Saved(saved) => Ok((saved.document, saved.damaged)),
            super::journal::Read::NoCheckpoint => {
                Err("Session journal contains no complete checkpoint; file left unchanged".into())
            }
        }
    }

    pub(super) fn journal_path(&self, id: &str) -> PathBuf {
        self.session_directory(id).join(layout::JOURNAL)
    }

    pub(super) fn summary_path(&self, id: &str) -> PathBuf {
        self.session_directory(id).join(layout::SUMMARY)
    }
}

/// The session a file of the earlier flat layout belongs to.
fn legacy_id(name: &str) -> Option<&str> {
    name.strip_suffix(".jsonl")
        .or_else(|| name.strip_suffix(".json"))
        .filter(|id| valid_id(id))
}

impl Lease {
    /// Saves a whole document, as fixtures and measurements do.
    #[cfg(test)]
    pub fn save(&self, document: &Document) -> Result<(), String> {
        self.save_tail(&super::journal::Tail::whole(document), &document.title())
    }

    /// Appends a checkpoint; `title` is the whole document's.
    pub fn save_tail(&self, tail: &super::journal::Tail<'_>, title: &str) -> Result<(), String> {
        let document = tail.document;
        if document.id != self.id || !same_directory(&document.directory, &self.store.directory) {
            return Err("Session identity or working directory changed".into());
        }
        super::journal::append(
            &self.store.journal_path(&self.id),
            tail,
            &mut self.journal.lock().unwrap(),
        )?;
        if let Some(stamp) = super::summary::stamp(&self.store, &self.id) {
            // A cache failure cannot turn a committed checkpoint into a failed save.
            let _ = super::summary::save(&self.store, document, title, &stamp);
        }
        Ok(())
    }

    /// Messages and events already journaled, once the journal was read.
    pub fn history(&self) -> Option<(usize, usize)> {
        self.journal
            .lock()
            .unwrap()
            .as_ref()
            .map(super::journal::Position::history)
    }
}

pub(super) fn reject_special(path: &Path) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if !metadata.is_file() || metadata.file_type().is_symlink() => {
            Err("Session files and locks must be regular files, not links".into())
        }
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("Could not access session file: {error}")),
    }
}

#[cfg(all(test, unix))]
mod lock_tests {
    use super::*;
    use crate::{effort::Effort, test_support::Directory};

    #[test]
    fn dropping_a_lease_releases_the_lock_while_a_duplicated_descriptor_is_open() {
        let home = Directory::new();
        let directory = Directory::new();
        let store = Store::new(home.path().to_path_buf(), directory.path()).unwrap();
        let doc = Document::new(
            directory.path().to_path_buf(),
            "fixture/model".into(),
            Effort::Default,
        )
        .unwrap();
        let lease = store.acquire(&doc.id).unwrap();
        let duplicate = lease._lock.try_clone().unwrap();
        assert!(store.acquire(&doc.id).is_err());
        drop(lease);
        let next = store.acquire(&doc.id).unwrap();
        assert!(store.acquire(&doc.id).is_err());
        drop(duplicate);
        assert!(store.acquire(&doc.id).is_err());
        drop(next);
        assert!(store.acquire(&doc.id).is_ok());
    }
}
