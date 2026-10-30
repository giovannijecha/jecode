use super::{Document, canonical, directory_key, same_directory, valid_id};
use std::collections::BTreeSet;
use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

#[derive(Clone)]
pub struct Store {
    pub(super) directory: PathBuf,
    pub(super) bucket: PathBuf,
    temporary: PathBuf,
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
    pub fn output_directory(&self) -> PathBuf {
        self.bucket.join("outputs")
    }

    /// Session-owned copies of attached files, shared by this project's sessions.
    pub fn attachments(&self) -> crate::attachments::Pool {
        crate::attachments::Pool::new(self.bucket.join("attachments"))
    }

    /// Where this project's session documents and journals live.
    pub fn bucket(&self) -> &Path {
        &self.bucket
    }

    /// The canonical project directory these sessions belong to.
    pub fn directory(&self) -> &Path {
        &self.directory
    }

    pub fn temporary_area(&self, id: &str) -> Result<crate::scratch::Area, String> {
        crate::scratch::Area::new(self.temporary.join(id), self.directory.clone(), id)
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
        // A stable owned bucket calculation avoids path-length dependent names.
        // The canonical directory in each document is the authoritative scope.
        let bucket = directory_key(&directory).bytes().fold(0u128, |key, byte| {
            key.wrapping_mul(257).wrapping_add(u128::from(byte) + 1)
        });
        Ok(Self {
            directory,
            bucket: home.join("sessions").join(format!("{bucket:032x}")),
            temporary: home.join("tmp").join(format!("{bucket:032x}")),
        })
    }

    pub fn list(&self) -> Result<Listing, String> {
        let entries = match fs::read_dir(&self.bucket) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Listing::default());
            }
            Err(error) => return Err(format!("Could not list sessions: {error}")),
        };
        let mut ids = BTreeSet::new();
        for entry in entries {
            let entry = entry.map_err(|error| format!("Could not list sessions: {error}"))?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if let Some(id) = name.strip_suffix(".jsonl")
                && valid_id(id)
            {
                ids.insert(id.to_string());
            }
        }
        let mut listing = Listing::default();
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

    pub(super) fn acquire(&self, id: &str) -> Result<Lease, String> {
        if !valid_id(id) {
            return Err("Invalid session identifier".into());
        }
        crate::disk::private_directory(&self.bucket)
            .map_err(|error| format!("Could not create session directory: {error}"))?;
        let path = self.bucket.join(format!("{id}.lock"));
        reject_special(&path)?;
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
        self.bucket.join(format!("{id}.jsonl"))
    }
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
