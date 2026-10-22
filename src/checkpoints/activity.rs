//! Live sessions using a workspace archive, so every session can see
//! concurrent activity. An entry is a held lock plus a small description;
//! the lock vanishes with its process, which marks the entry stale.
use super::files;
use super::locks::Exclusive;
use crate::json::{self, Value};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub origin: String,
    pub pid: u32,
    pub started: u64,
    pub state: String,
    pub updated: u64,
}

pub struct Registration {
    archive: PathBuf,
    description: PathBuf,
    entry: Mutex<Entry>,
    _lock: Exclusive,
}

fn directory(archive: &Path) -> PathBuf {
    super::layout::activity_directory(archive)
}

pub fn register(archive: &Path, origin: &str) -> Result<Registration, String> {
    let name = files::unique("session");
    let directory = directory(archive);
    let lock = Exclusive::acquire(&directory.join(format!("{name}.lock")))
        .map_err(|error| format!("Could not register session activity: {error}"))?;
    let now = crate::sessions::now();
    let registration = Registration {
        archive: archive.to_path_buf(),
        description: directory.join(format!("{name}.json")),
        entry: Mutex::new(Entry {
            origin: origin.into(),
            pid: std::process::id(),
            started: now,
            state: "idle".into(),
            updated: now,
        }),
        _lock: lock,
    };
    registration.save();
    Ok(registration)
}

impl Registration {
    /// Describes what the session is doing; best effort.
    pub fn update(&self, state: &str) {
        {
            let mut entry = self.entry.lock().unwrap_or_else(|e| e.into_inner());
            entry.state = state.chars().take(160).collect();
            entry.updated = crate::sessions::now();
        }
        self.save();
    }

    fn save(&self) {
        let entry = self.entry.lock().unwrap_or_else(|e| e.into_inner()).clone();
        let text = Value::object([
            ("origin", Value::string(entry.origin)),
            ("pid", Value::number(entry.pid)),
            ("started", Value::number(entry.started)),
            ("state", Value::string(entry.state)),
            ("updated", Value::number(entry.updated)),
        ])
        .encode();
        let Ok(_store) = Exclusive::acquire(&super::layout::store_lock(&self.archive)) else {
            return;
        };
        // Atomic replacement briefly owns both descriptions. Admission is
        // serialized with journal and compaction writes in other processes.
        // Activity descriptions are tiny and never count as history payload.
        // A crashed session's description is removed with its lock anyway.
        let _ = files::replace_unsynced(&self.description, text.as_bytes());
    }
}

impl Drop for Registration {
    fn drop(&mut self) {
        let Ok(_store) = Exclusive::acquire(&super::layout::store_lock(&self.archive)) else {
            return;
        };
        let _ = std::fs::remove_file(&self.description);
    }
}

/// Live entries; stale descriptions of exited processes are removed.
pub fn list(archive: &Path) -> Vec<Entry> {
    let Ok(_store) = Exclusive::acquire(&super::layout::store_lock(archive)) else {
        return Vec::new();
    };
    let directory = directory(archive);
    let Ok(entries) = std::fs::read_dir(&directory) else {
        return Vec::new();
    };
    let mut live = Vec::new();
    for file in entries.flatten() {
        let path = file.path();
        if path.extension().and_then(|extension| extension.to_str()) != Some("lock") {
            continue;
        }
        let description = path.with_extension("json");
        // A new session creates its lock file just before locking it.
        let recent = file
            .metadata()
            .and_then(|metadata| metadata.modified())
            .ok()
            .and_then(|modified| modified.elapsed().ok())
            .is_none_or(|age| age < std::time::Duration::from_secs(10));
        match Exclusive::try_acquire(&path) {
            Ok(Some(_)) if recent => {}
            Ok(Some(held)) => {
                // Only an exited process leaves its lock free.
                drop(held);
                let _ = std::fs::remove_file(&description);
                let _ = std::fs::remove_file(&path);
            }
            Ok(None) => {
                if let Some(entry) = std::fs::read_to_string(&description)
                    .ok()
                    .and_then(|text| json::parse(&text).ok())
                    .and_then(|value| parse(&value))
                {
                    live.push(entry);
                }
            }
            Err(_) => {}
        }
    }
    live.sort_by_key(|entry| entry.started);
    live
}

fn parse(value: &Value) -> Option<Entry> {
    let number = |key: &str| value.get(key)?.as_usize().map(|number| number as u64);
    Some(Entry {
        origin: value.get("origin")?.as_str()?.into(),
        pid: number("pid")? as u32,
        started: number("started")?,
        state: value.get("state")?.as_str()?.into(),
        updated: number("updated")?,
    })
}
