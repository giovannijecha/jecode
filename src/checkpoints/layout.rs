//! Archive paths. Version 1 archives use flat paths until idle migration;
//! version 2 puts disposable data under `history/` and runtime state under
//! `runtime/`. Identity and user policy remain at the archive root.
use crate::json::{self, Value};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

static VERSION_TWO: OnceLock<Mutex<HashSet<PathBuf>>> = OnceLock::new();
pub const LEGACY_SENTINEL: &[u8] = b"migrated-to-history";

pub fn history(archive: &Path) -> PathBuf {
    if cached_version(archive) == Some(2) {
        archive.join("history")
    } else {
        archive.to_path_buf()
    }
}

pub fn current(archive: &Path) -> PathBuf {
    history(archive).join("CURRENT")
}

pub fn generation_directory(archive: &Path, generation: u64) -> PathBuf {
    history(archive).join(format!("gen-{generation:06}"))
}

pub fn generation_lock(archive: &Path, generation: u64) -> PathBuf {
    history(archive).join(format!("gen-{generation:06}.lock"))
}

pub fn store_lock(archive: &Path) -> PathBuf {
    // This lock is stable across an in-place v1 to v2 migration. A previous
    // binary also coordinates through it until the root CURRENT sentinel
    // rejects its attempt to read the migrated archive.
    archive.join("store.lock")
}

pub fn gc_lock(archive: &Path) -> PathBuf {
    archive.join("gc.lock")
}

pub fn owners_directory(archive: &Path) -> PathBuf {
    runtime(archive).join("owners")
}

pub fn owner_lock(archive: &Path, owner: &str) -> PathBuf {
    owners_directory(archive).join(format!("{owner}.lock"))
}

pub fn activity_directory(archive: &Path) -> PathBuf {
    runtime(archive).join("activity")
}

pub fn leases_directory(archive: &Path) -> PathBuf {
    runtime(archive).join("leases")
}

pub fn runtime(archive: &Path) -> PathBuf {
    if cached_version(archive) == Some(2) {
        archive.join("runtime")
    } else {
        archive.to_path_buf()
    }
}

pub fn version(archive: &Path) -> Option<u64> {
    let text = fs::read_to_string(archive.join("workspace.json")).ok()?;
    let value = json::parse(&text).ok()?;
    match value.get("version")? {
        Value::Number(number) => number.parse().ok(),
        _ => None,
    }
}

fn cached_version(archive: &Path) -> Option<u64> {
    let cache = VERSION_TWO.get_or_init(|| Mutex::new(HashSet::new()));
    if cache
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .contains(archive)
    {
        return Some(2);
    }
    // Only version 2 is immutable. Version 1 may migrate in another process.
    let version = version(archive)?;
    if version == 2 {
        cache
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .insert(archive.to_path_buf());
    }
    Some(version)
}

/// Call before lifecycle admission. In an existing v2 archive, the stable
/// store lock serializes this check with an older binary opening a flat journal.
pub fn guard_legacy(archive: &Path) -> Result<(), String> {
    let _store = super::locks::Exclusive::acquire(&store_lock(archive))
        .map_err(|error| format!("Could not guard checkpoint layout: {error}"))?;
    guard_legacy_locked(archive)
}

/// Called while the stable store lock is held, including during expiry.
pub fn guard_legacy_locked(archive: &Path) -> Result<(), String> {
    for entry in fs::read_dir(archive).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        if entry.file_name().to_string_lossy().starts_with("gen-") {
            return Err(
                "Legacy checkpoint history exists beside history/; both histories were preserved"
                    .into(),
            );
        }
    }
    let current = archive.join("CURRENT");
    match fs::symlink_metadata(&current) {
        Ok(metadata) if metadata.is_file() && !metadata.file_type().is_symlink() => {
            if fs::read(&current).map_err(|error| error.to_string())? == LEGACY_SENTINEL {
                // A rejected older opener can leave an empty root owners/.
                return Ok(());
            }
            return Err(
                "A legacy checkpoint CURRENT exists beside history/; history was preserved".into(),
            );
        }
        Ok(_) => return Err("Checkpoint compatibility marker is not an ordinary file".into()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.to_string()),
    }
    let owners = archive.join("owners");
    match fs::symlink_metadata(&owners) {
        Ok(metadata)
            if !metadata.is_dir()
                || metadata.file_type().is_symlink()
                || fs::read_dir(&owners)
                    .map_err(|error| error.to_string())?
                    .next()
                    .is_some() =>
        {
            return Err("Legacy checkpoint owners exist beside history/; close older sessions and preserve their history".into());
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.to_string()),
    }
    super::files::atomic_write(&current, LEGACY_SENTINEL)
        .map_err(|error| format!("Could not guard checkpoint layout: {error}"))
}
