//! Inactive archive expiry. A Store holds a shared lease throughout its life;
//! expiry takes the exclusive lease and never removes a live reader's history.
use super::files;
use super::identity::Location;
use super::index::Index;
use super::journal::Journal;
use super::layout;
use super::locks::{Exclusive, Shared};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

// sessions::now() and persisted activity timestamps are Unix milliseconds.
const RETENTION_MILLIS: u64 = 7 * 24 * 60 * 60 * 1_000;
const SWEEP_MILLIS: u64 = 60 * 60 * 1_000;
static LAST_SWEEP: OnceLock<Mutex<HashMap<PathBuf, u64>>> = OnceLock::new();
static LAST_TOUCH: OnceLock<Mutex<HashMap<PathBuf, u64>>> = OnceLock::new();
mod migration;

pub struct Lease {
    _held: Shared,
}

fn lease_path(archive: &Path) -> PathBuf {
    archive.join("runtime/lifecycle.lock")
}

fn last_use_path(archive: &Path) -> PathBuf {
    archive.join("runtime/last-use")
}

fn high_water_path(archive: &Path) -> PathBuf {
    archive.join("runtime/high-water")
}

fn expiry_pending_path(archive: &Path) -> PathBuf {
    archive.join("runtime/expiry-pending")
}

/// Admits a Store without counting background maintenance as user activity.
/// An expired archive is examined before a new shared reader is admitted.
pub fn open(location: &Location) -> Result<Lease, String> {
    let archive = &location.directory;
    let runtime = archive.join("runtime");
    if runtime.exists() && !ordinary_directory(&runtime) {
        return Err("Checkpoint runtime directory is not an ordinary directory".into());
    }
    let history = archive.join("history");
    if history.exists() && !ordinary_directory(&history) {
        return Err("Checkpoint history directory is not an ordinary directory".into());
    }
    for path in [
        layout::owners_directory(archive),
        layout::leases_directory(archive),
        layout::activity_directory(archive),
    ] {
        if path.exists() && !ordinary_directory(&path) {
            return Err(format!(
                "Checkpoint runtime entry {} is not an ordinary directory",
                path.display()
            ));
        }
    }
    if let Some(exclusive) = Exclusive::try_acquire(&lease_path(archive))
        .map_err(|error| format!("Could not lock checkpoint lifecycle: {error}"))?
    {
        if let Err(error) = expire(archive)
            && (archive.join(".purging").exists()
                || archive.join(".migrating").exists()
                || expiry_pending_path(archive).exists())
        {
            return Err(error);
        }
        drop(exclusive);
    }
    let held = Shared::acquire(&lease_path(archive))
        .map_err(|error| format!("Could not join checkpoint archive: {error}"))?;
    // A sweeper may finish (or fail) between the nonblocking exclusive probe
    // and this shared lock. Never admit a reader into an incomplete purge.
    if archive.join(".purging").exists()
        || archive.join(".migrating").exists()
        || expiry_pending_path(archive).exists()
    {
        return Err("Checkpoint archive recovery is pending".into());
    }
    Ok(Lease { _held: held })
}

/// Records actual user access. Call only after opening a user session or
/// processing a user action, never during maintenance or collection.
pub fn touch(archive: &Path) -> Result<(), String> {
    let _lock = Exclusive::acquire(&archive.join("runtime/last-use.lock"))
        .map_err(|error| format!("Could not lock checkpoint activity: {error}"))?;
    let now = crate::sessions::now();
    // A damaged activity timestamp must never block an explicit user session.
    // Its repair is a fresh activity event, while the sweeper fails closed.
    let previous = read_number(&last_use_path(archive))
        .ok()
        .flatten()
        .unwrap_or(0);
    if now > previous {
        files::atomic_write(&last_use_path(archive), now.to_string().as_bytes())
            .map_err(|error| format!("Could not record checkpoint activity: {error}"))?;
    }
    LAST_TOUCH
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .insert(archive.to_path_buf(), now);
    Ok(())
}

/// User actions renew activity at most hourly per process. The durable time
/// remains exact enough for a seven-day boundary without a write per edit.
pub fn touch_throttled(archive: &Path) -> Result<(), String> {
    let now = crate::sessions::now();
    let recent = LAST_TOUCH
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .get(archive)
        .is_some_and(|last| now.saturating_sub(*last) < SWEEP_MILLIS);
    if recent { Ok(()) } else { touch(archive) }
}

pub fn high_water(archive: &Path) -> Result<u64, String> {
    Ok(read_number(&high_water_path(archive))?.unwrap_or(0))
}

/// Sweep is best effort per archive: an inaccessible or damaged archive is
/// preserved. The caller can still use other workspaces.
pub fn sweep(archives: &Path) -> Result<usize, String> {
    let entries = match fs::read_dir(archives) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(error) => return Err(format!("Could not inspect checkpoint archives: {error}")),
    };
    let mut removed = 0;
    for entry in entries.flatten() {
        let archive = entry.path();
        if !ordinary_directory(&archive) || !super::identity::recognized_archive(&archive) {
            continue;
        }
        let runtime = archive.join("runtime");
        if runtime.exists() && !ordinary_directory(&runtime) {
            continue;
        }
        let Ok(Some(_lease)) = Exclusive::try_acquire(&lease_path(&archive)) else {
            continue;
        };
        if expire(&archive).unwrap_or(false) {
            removed += 1;
        }
    }
    Ok(removed)
}

pub fn sweep_throttled(archives: &Path) -> Result<usize, String> {
    let now = crate::sessions::now();
    {
        let mut swept = LAST_SWEEP
            .get_or_init(|| Mutex::new(HashMap::new()))
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if swept
            .get(archives)
            .is_some_and(|previous| now.saturating_sub(*previous) < SWEEP_MILLIS)
        {
            return Ok(0);
        }
        swept.insert(archives.to_path_buf(), now);
    }
    let result = sweep(archives);
    if result.is_err() {
        LAST_SWEEP
            .get_or_init(|| Mutex::new(HashMap::new()))
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .remove(archives);
    }
    result
}

fn expire(archive: &Path) -> Result<bool, String> {
    if !ordinary_directory(archive) || !super::identity::recognized_archive(archive) {
        return Ok(false);
    }
    let _store = Exclusive::acquire(&layout::store_lock(archive))
        .map_err(|error| format!("Could not lock checkpoint archive: {error}"))?;
    resume_interrupted_purge(archive)?;
    let observed = read_number(&last_use_path(archive))?;
    let last_use = if let Some(last_use) = observed {
        last_use
    } else {
        // Old archives did not record user activity. Their first observation
        // starts a full grace period rather than guessing from file mtimes.
        let now = crate::sessions::now();
        files::atomic_write(&last_use_path(archive), now.to_string().as_bytes())
            .map_err(|error| format!("Could not record checkpoint grace period: {error}"))?;
        now
    };
    if !migration::resume_or_start(archive)? || observed.is_none() {
        return Ok(false);
    }
    // A process can coalesce user actions for one hour. Waiting that extra
    // hour prevents an action just before a crash from expiring early.
    if crate::sessions::now().saturating_sub(last_use) < RETENTION_MILLIS + SWEEP_MILLIS {
        return Ok(false);
    }
    if !has_history(archive)? || old_owner_active(archive)? {
        return Ok(false);
    }
    // The lease stops new Stores; the generation locks also protect legacy
    // readers that opened before lifecycle leases existed.
    let mut journal = Journal::open(archive)?;
    let mut index = Index::default();
    journal.catch_up(&mut |entry| index.apply(entry))?;
    let mut highest = index.next;
    let mut followed = HashSet::new();
    while let Some(generation) = index.moved {
        if !journal.damage.is_empty() || journal.blocked.is_some() || !followed.insert(generation) {
            return Ok(false);
        }
        journal = Journal::open_existing_generation(archive, generation)?;
        index = Index::default();
        journal.catch_up(&mut |entry| index.apply(entry))?;
        highest = highest.max(index.next);
    }
    if !index.pins.is_empty()
        || !index.pending.is_empty()
        || !journal.damage.is_empty()
        || journal.blocked.is_some()
    {
        return Ok(false);
    }
    drop(journal);
    let generation_locks = generation_locks(archive)?;
    let mut held = Vec::new();
    for path in generation_locks {
        match Exclusive::try_acquire(&path)
            .map_err(|error| format!("Could not inspect checkpoint reader: {error}"))?
        {
            Some(lock) => held.push(lock),
            None => return Ok(false),
        }
    }
    let next = highest.max(high_water(archive)?);
    files::atomic_write(&high_water_path(archive), next.to_string().as_bytes())
        .map_err(|error| format!("Could not preserve checkpoint sequence: {error}"))?;
    drop(held);
    remove_history(archive)?;
    Ok(true)
}

fn read_number(path: &Path) -> Result<Option<u64>, String> {
    match fs::read_to_string(path) {
        Ok(text) => text
            .trim()
            .parse()
            .map(Some)
            .map_err(|_| format!("Invalid checkpoint metadata at {}", path.display())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!("Could not read checkpoint metadata: {error}")),
    }
}

fn ordinary_directory(path: &Path) -> bool {
    fs::symlink_metadata(path)
        .is_ok_and(|metadata| metadata.is_dir() && !metadata.file_type().is_symlink())
}

fn has_history(archive: &Path) -> Result<bool, String> {
    if layout::version(archive) == Some(2) {
        let path = layout::history(archive);
        return Ok(ordinary_directory(&path));
    }
    let entries = fs::read_dir(archive)
        .map_err(|error| format!("Could not inspect checkpoint history: {error}"))?;
    let mut found = false;
    for entry in entries {
        let entry =
            entry.map_err(|error| format!("Could not inspect checkpoint history: {error}"))?;
        found |= is_generation(&entry.path());
    }
    Ok(found)
}

fn is_generation(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    name.strip_prefix("gen-").is_some_and(|digits| {
        !digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit())
    }) && ordinary_directory(path)
}

fn generation_locks(archive: &Path) -> Result<Vec<PathBuf>, String> {
    let history = layout::history(archive);
    let entries = fs::read_dir(history)
        .map_err(|error| format!("Could not inspect checkpoint generations: {error}"))?;
    let mut paths = Vec::new();
    for entry in entries {
        let entry =
            entry.map_err(|error| format!("Could not inspect checkpoint generation: {error}"))?;
        let path = entry.path();
        if is_generation(&path) {
            let name = path.file_name().unwrap().to_string_lossy();
            let generation = name[4..]
                .parse::<u64>()
                .map_err(|_| "Invalid checkpoint generation".to_string())?;
            paths.push(layout::generation_lock(archive, generation));
        }
    }
    paths.sort();
    Ok(paths)
}

fn old_owner_active(archive: &Path) -> Result<bool, String> {
    let directory = layout::owners_directory(archive);
    if directory.exists() && !ordinary_directory(&directory) {
        return Ok(true);
    }
    let entries = match fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(format!("Could not inspect checkpoint owners: {error}")),
    };
    for entry in entries {
        let entry =
            entry.map_err(|error| format!("Could not inspect checkpoint owner: {error}"))?;
        let path = entry.path();
        if path.extension().is_none_or(|extension| extension != "lock") {
            continue;
        }
        if fs::symlink_metadata(&path).map_or(true, |metadata| {
            !metadata.is_file() || metadata.file_type().is_symlink()
        }) {
            return Ok(true);
        }
        match Exclusive::try_acquire(&path) {
            Ok(Some(_)) => {}
            Ok(None) | Err(_) => return Ok(true),
        }
    }
    Ok(false)
}

fn remove_history(archive: &Path) -> Result<(), String> {
    if layout::version(archive) == Some(2) {
        files::atomic_write(&expiry_pending_path(archive), b"1")
            .map_err(|error| format!("Could not mark checkpoint expiry: {error}"))?;
        finish_v2_purge(archive)?;
    } else {
        // A marker lets the next opener complete a partially interrupted
        // legacy purge before the journal can be read again.
        files::atomic_write(&archive.join(".purging"), b"1")
            .map_err(|error| format!("Could not mark checkpoint expiry: {error}"))?;
        resume_interrupted_purge(archive)?;
    }
    Ok(())
}

fn resume_interrupted_purge(archive: &Path) -> Result<(), String> {
    let pending = expiry_pending_path(archive);
    if pending.exists() {
        if layout::version(archive) != Some(2) {
            return Err("Checkpoint expiry marker has an unsupported layout".into());
        }
        let valid_marker = fs::symlink_metadata(&pending)
            .is_ok_and(|metadata| metadata.is_file() && !metadata.file_type().is_symlink())
            && fs::read(&pending).is_ok_and(|content| content == b"1");
        if !valid_marker || read_number(&high_water_path(archive))?.is_none() {
            return Err("Checkpoint expiry marker is incomplete or damaged".into());
        }
        finish_v2_purge(archive)?;
    }
    let marker = archive.join(".purging");
    if !marker.exists() {
        return Ok(());
    }
    if old_owner_active(archive)? {
        return Err("Checkpoint expiry is waiting for a legacy reader to exit".into());
    }
    for path in generation_locks(archive)? {
        if Exclusive::try_acquire(&path)
            .map_err(|error| format!("Could not inspect checkpoint reader: {error}"))?
            .is_none()
        {
            return Err("Checkpoint expiry is waiting for a reader to exit".into());
        }
    }
    let entries = fs::read_dir(archive)
        .map_err(|error| format!("Could not finish checkpoint expiry: {error}"))?;
    for entry in entries.flatten() {
        let path = entry.path();
        if is_generation(&path) {
            fs::remove_dir_all(&path)
                .map_err(|error| format!("Could not remove expired generation: {error}"))?;
        } else if path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| {
                name == "CURRENT" || (name.starts_with("gen-") && name.ends_with(".lock"))
            })
        {
            fs::remove_file(&path)
                .map_err(|error| format!("Could not remove expired generation marker: {error}"))?;
        }
    }
    fs::remove_file(marker)
        .map_err(|error| format!("Could not finish checkpoint expiry: {error}"))?;
    Ok(())
}

fn finish_v2_purge(archive: &Path) -> Result<(), String> {
    let history = layout::history(archive);
    let detached = archive.join("history.expired");
    if detached.exists() {
        if !ordinary_directory(&detached) {
            return Err("Could not safely inspect detached checkpoint history".into());
        }
        fs::remove_dir_all(&detached)
            .map_err(|error| format!("Could not finish checkpoint expiry: {error}"))?;
    }
    if history.exists() {
        if !ordinary_directory(&history) {
            return Err("Could not safely inspect expired checkpoint history".into());
        }
        fs::rename(&history, &detached)
            .map_err(|error| format!("Could not detach expired history: {error}"))?;
        files::sync_directory(archive)
            .map_err(|error| format!("Could not flush checkpoint expiry: {error}"))?;
        fs::remove_dir_all(&detached)
            .map_err(|error| format!("Could not remove expired history: {error}"))?;
    }
    fs::remove_file(expiry_pending_path(archive))
        .map_err(|error| format!("Could not finish checkpoint expiry: {error}"))?;
    files::sync_directory(&archive.join("runtime"))
        .map_err(|error| format!("Could not flush checkpoint expiry: {error}"))?;
    Ok(())
}

#[cfg(test)]
mod tests;
