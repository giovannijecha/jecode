use super::{Document, canonical, directory_key, identifier, same_directory, valid_id};
use crate::json;
use std::collections::BTreeSet;
use std::fs::{self, File, OpenOptions};
use std::io::Read;
#[cfg(test)]
use std::io::Write;
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
    pub model: String,
}

impl Summary {
    pub fn description(&self) -> String {
        let age = super::now().saturating_sub(self.updated) / 1000;
        let age = match age {
            0..60 => "just now".into(),
            60..3600 => format!("{}m ago", age / 60),
            3600..86400 => format!("{}h ago", age / 3600),
            _ => format!("{}d ago", age / 86400),
        };
        format!("{age} · {} · {}", self.model, self.id)
    }
}

#[derive(Default)]
pub struct Listing {
    pub sessions: Vec<Summary>,
    pub warnings: Vec<String>,
}

pub(super) struct Lease {
    // The lock covers the append journal and any legacy migration.
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

struct ReadFailure {
    message: String,
    recoverable: bool,
}

#[derive(PartialEq, Eq)]
enum Origin {
    Journal,
    Legacy,
}
impl From<String> for ReadFailure {
    fn from(message: String) -> Self {
        Self {
            message,
            recoverable: true,
        }
    }
}
impl From<&str> for ReadFailure {
    fn from(message: &str) -> Self {
        message.to_string().into()
    }
}
impl ReadFailure {
    fn incompatible(message: &str) -> Self {
        Self {
            message: message.into(),
            recoverable: false,
        }
    }
}

impl Store {
    pub fn output_directory(&self) -> PathBuf {
        self.bucket.join("outputs")
    }

    pub fn temporary_area(&self, id: &str) -> Result<crate::scratch::Area, String> {
        crate::scratch::Area::new(self.temporary.join(id), self.directory.clone(), id)
    }

    pub(super) fn preserve_legacy_damage(&self, id: &str) -> Result<(), String> {
        if self.journal_path(id).exists() {
            return Ok(());
        }
        let path = self.path(id);
        if path.exists() && self.read(&path, id).is_err() {
            let damaged = path.with_extension(format!("json.damaged-{}", identifier()));
            fs::copy(&path, damaged)
                .map_err(|error| format!("Could not preserve damaged legacy session: {error}"))?;
        }
        Ok(())
    }

    #[cfg(test)]
    pub fn fixture_load(&self, id: &str) -> Result<Document, String> {
        self.load(id).map(|(doc, _)| doc)
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
            if let Some(id) = name
                .strip_suffix(".json")
                .or_else(|| name.strip_suffix(".jsonl"))
                .or_else(|| name.strip_suffix(".json.bak"))
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
            match self.load_with_origin(&id) {
                Ok((document, _, origin)) => {
                    if origin == Origin::Journal
                        && let Some(stamp) = &stamp
                    {
                        // The cache is optional; opening still validates the journal itself.
                        let _ = super::summary::save(self, &document, stamp);
                    }
                    listing.sessions.push(Summary {
                        title: document.title(),
                        updated: document.updated,
                        model: document.model,
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
        private_directory(&self.bucket)
            .map_err(|error| format!("Could not create session directory: {error}"))?;
        let path = self.bucket.join(format!("{id}.lock"));
        reject_special(&path)?;
        let lock = options()
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
        let path = self.path(id);
        if path.exists()
            || path.with_extension("json.bak").exists()
            || self.journal_path(id).exists()
        {
            return Err("Session identifier already exists; saved data was left unchanged".into());
        }
        Ok(lease)
    }

    pub(super) fn load(&self, id: &str) -> Result<(Document, bool), String> {
        self.load_with_origin(id)
            .map(|(document, backup, _)| (document, backup))
    }

    fn load_with_origin(&self, id: &str) -> Result<(Document, bool, Origin), String> {
        if !valid_id(id) {
            return Err("Invalid session identifier".into());
        }
        let journal = self.journal_path(id);
        if journal.exists() {
            return match super::journal::read(&journal, id, &self.directory)? {
                super::journal::Read::Saved(saved) => {
                    let saved = *saved;
                    Ok((saved.document, saved.damaged, Origin::Journal))
                }
                super::journal::Read::NoCheckpoint => self
                    .load_legacy(id)
                    .map(|(document, backup)| (document, backup, Origin::Legacy))
                    .map_err(|_| {
                        "Session journal contains no complete checkpoint; file left unchanged"
                            .into()
                    }),
            };
        }
        self.load_legacy(id)
            .map(|(document, backup)| (document, backup, Origin::Legacy))
    }

    fn load_legacy(&self, id: &str) -> Result<(Document, bool), String> {
        let path = self.path(id);
        if !path.exists() && !path.with_extension("json.bak").exists() {
            return Err("Session not found in this working directory".into());
        }
        match self.read(&path, id) {
            Ok(document) => Ok((document, false)),
            Err(error) if !error.recoverable => Err(error.message),
            Err(error) => self
                .read(&path.with_extension("json.bak"), id)
                .map(|document| (document, true))
                .map_err(|_| error.message),
        }
    }

    fn read(&self, path: &Path, id: &str) -> Result<Document, ReadFailure> {
        reject_special(path)?;
        let metadata =
            fs::metadata(path).map_err(|error| format!("Could not read session: {error}"))?;
        if !metadata.is_file() {
            return Err("Session must be a regular file".into());
        }
        let mut text = String::new();
        File::open(path)
            .and_then(|mut file| file.read_to_string(&mut text))
            .map_err(|error| format!("Could not read session: {error}"))?;
        let value = json::parse(&text)?;
        if value
            .get("format")
            .is_some_and(|format| format.as_str() != Some("jecode.session"))
            || value
                .get("format_version")
                .is_some_and(|version| version != &crate::json::Value::number(1))
        {
            return Err(ReadFailure::incompatible(
                "Unsupported session format or version; file left unchanged",
            ));
        }
        let document = Document::parse(&value)?;
        if document.id != id {
            return Err(ReadFailure::incompatible(
                "Session filename and identifier differ",
            ));
        }
        if !same_directory(&document.directory, &self.directory) {
            return Err(ReadFailure::incompatible(
                "Session belongs to another working directory",
            ));
        }
        Ok(document)
    }

    fn path(&self, id: &str) -> PathBuf {
        self.bucket.join(format!("{id}.json"))
    }

    pub(super) fn journal_path(&self, id: &str) -> PathBuf {
        self.bucket.join(format!("{id}.jsonl"))
    }
}

impl Lease {
    pub fn save(&self, document: &Document) -> Result<(), String> {
        if document.id != self.id || !same_directory(&document.directory, &self.store.directory) {
            return Err("Session identity or working directory changed".into());
        }
        super::journal::append(
            &self.store.journal_path(&self.id),
            document,
            &mut self.journal.lock().unwrap(),
        )?;
        if let Some(stamp) = super::summary::stamp(&self.store, &self.id) {
            // A cache failure cannot turn a committed checkpoint into a failed save.
            let _ = super::summary::save(&self.store, document, &stamp);
        }
        Ok(())
    }

    #[cfg(test)]
    pub fn save_legacy(&self, document: &Document) -> Result<(), String> {
        let bytes = format!("{}\n", document.value().pretty()).into_bytes();
        let path = self.store.path(&self.id);
        reject_special(&path)?;
        if path.exists() {
            let metadata = fs::metadata(&path).map_err(|error| error.to_string())?;
            if metadata.permissions().readonly() {
                return Err("Session file is read-only".into());
            }
            let old = fs::read(&path)
                .map_err(|error| format!("Could not preserve saved session: {error}"))?;
            match self.store.read(&path, &self.id) {
                Ok(_) => replace(&path.with_extension("json.bak"), &old)?,
                Err(error) if !error.recoverable => return Err(error.message),
                Err(_) => {
                    // Preserve damaged input before repairing from the last valid copy.
                    let damaged = path.with_extension(format!("json.damaged-{}", identifier()));
                    replace(&damaged, &old)?;
                }
            }
        }
        replace(&path, &bytes)
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

pub(super) fn options() -> OpenOptions {
    #[allow(unused_mut)]
    let mut options = OpenOptions::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options
}

fn private_directory(path: &Path) -> std::io::Result<()> {
    if !path.exists() {
        fs::create_dir_all(path)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
        }
    }
    Ok(())
}

#[cfg(test)]
fn replace(path: &Path, bytes: &[u8]) -> Result<(), String> {
    reject_special(path)?;
    let temporary = path.with_extension(format!("tmp-{}", identifier()));
    let mut file = options()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .map_err(|error| format!("Could not prepare session save: {error}"))?;
    let result = file.write_all(bytes).and_then(|_| file.sync_all());
    drop(file);
    let result = result.and_then(|_| fs::rename(&temporary, path));
    if let Err(error) = result {
        let _ = fs::remove_file(&temporary);
        return Err(format!("Could not save session: {error}"));
    }
    Ok(())
}

#[cfg(test)]
pub(super) fn file_path(store: &Store, id: &str) -> PathBuf {
    let journal = store.journal_path(id);
    if journal.exists() {
        journal
    } else {
        store.path(id)
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
