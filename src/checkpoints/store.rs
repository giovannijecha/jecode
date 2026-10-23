//! Independent immutable recovery packs for modifying calls.
use super::files;
use super::format;
use super::hash;
use super::locks;
use super::pressure::{self, Pressure};
use super::state::State;
use super::types::{Snapshot, Summary, TTL_MILLIS};
use crate::cancel::Cancellation;
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
#[cfg(test)]
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

mod identity;
use identity::{add_exclusion, ordinary_directory, ordinary_file, recognized_workspace};

pub struct Store {
    pub root: PathBuf,
    pub directory: PathBuf,
    pub excluded: Vec<String>,
    pub leases: Arc<locks::Leases>,
    pressure: Mutex<Option<Arc<Pressure>>>,
    #[cfg(test)]
    pub(super) cancel_after_chunk: AtomicBool,
}

impl Store {
    pub fn open(home: &Path, root: &Path) -> Result<Arc<Self>, String> {
        let root =
            fs::canonicalize(root).map_err(|e| format!("Could not locate workspace: {e}"))?;
        if !root.is_dir() {
            return Err("Workspace is not a directory".into());
        }
        let directory = identity::open_workspace(home, &root)?;
        let home =
            fs::canonicalize(home).map_err(|e| format!("Could not locate recovery home: {e}"))?;
        let mut excluded = Vec::new();
        // Exclude every checkpoint archive if the workspace contains the user's
        // home. The logical names cover symlink aliases; canonical targets
        // cover storage physically located elsewhere inside the workspace.
        for path in [
            directory.parent().unwrap().to_path_buf(),
            home.join("checkpoints"),
            directory.clone(),
        ] {
            add_exclusion(&root, &path, &mut excluded);
        }
        excluded.sort();
        excluded.dedup();
        let leases = locks::Leases::new(directory.join(".internal/leases"));
        Ok(Arc::new(Self {
            root,
            directory,
            excluded,
            leases,
            pressure: Mutex::new(None),
            #[cfg(test)]
            cancel_after_chunk: AtomicBool::new(false),
        }))
    }

    pub(super) fn claim(&self, bytes: u64, cancel: &Cancellation) -> Result<(), String> {
        let mut cached = self.pressure.lock().unwrap_or_else(|e| e.into_inner());
        let pressure = match &*cached {
            Some(pressure) => Arc::clone(pressure),
            None => {
                let pressure = pressure::for_archive(&self.directory)?;
                *cached = Some(Arc::clone(&pressure));
                pressure
            }
        };
        drop(cached);
        pressure.claim(
            &self.directory,
            bytes,
            pressure::DEFAULT_MIN_FREE_BYTES,
            Some(cancel),
        )
    }

    pub fn load(&self, reference: &str, cancel: &Cancellation) -> Result<Loaded, String> {
        if cancel.requested() {
            return Err("Operation cancelled".into());
        }
        if !valid_reference(reference) {
            return Err("Invalid recovery reference".into());
        }
        let path = self.directory.join(format!("{reference}.rcp"));
        if !ordinary_file(&path)? {
            return Err("Recovery pack is missing or is not an ordinary file".into());
        }
        let file =
            File::open(&path).map_err(|e| format!("Could not open recovery {reference}: {e}"))?;
        loop {
            if cancel.requested() {
                return Err("Operation cancelled".into());
            }
            match file.try_lock_shared() {
                Ok(()) => break,
                Err(std::fs::TryLockError::WouldBlock) => {
                    cancel.wait(std::time::Duration::from_millis(10))?;
                }
                Err(std::fs::TryLockError::Error(error)) => {
                    return Err(format!("Could not lock recovery {reference}: {error}"));
                }
            }
        }
        let mut lease = ReadLease { file };
        let snapshot = format::read(&mut lease.file)?;
        if snapshot.reference != reference
            || snapshot.created != created_from_reference(reference).unwrap()
        {
            return Err("Recovery reference mismatch".into());
        }
        if snapshot.expires() <= super::clock::now() {
            return Err("Recovery has expired".into());
        }
        if cancel.requested() {
            return Err("Operation cancelled".into());
        }
        Ok(Loaded {
            snapshot,
            lease: Arc::new(lease),
        })
    }

    pub fn list(&self, limit: usize, cancel: &Cancellation) -> Result<Vec<Summary>, String> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        let now = super::clock::now();
        let mut names = Vec::new();
        for entry in fs::read_dir(&self.directory).map_err(|e| e.to_string())? {
            if cancel.requested() {
                return Err("Operation cancelled".into());
            }
            let entry = entry.map_err(|e| e.to_string())?;
            let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
                continue;
            };
            let Some(reference) = name.strip_suffix(".rcp") else {
                continue;
            };
            let Some(created) = created_from_reference(reference) else {
                continue;
            };
            if created.saturating_add(TTL_MILLIS) <= now {
                continue;
            }
            names.push((created, reference.to_owned()));
        }
        names.sort_unstable_by(|a, b| b.cmp(a));
        let mut result = Vec::new();
        for (_, reference) in names {
            if result.len() == limit {
                break;
            }
            if cancel.requested() {
                return Err("Operation cancelled".into());
            }
            let loaded = match self.load(&reference, cancel) {
                Ok(loaded) => loaded,
                Err(error) if error == "Operation cancelled" => return Err(error),
                Err(_) => continue,
            };
            let snapshot = &loaded.snapshot;
            result.push(Summary {
                reference: snapshot.reference.clone(),
                created: snapshot.created,
                tool: snapshot.tool.clone(),
                saved: snapshot
                    .entries
                    .values()
                    .filter(|s| s.recoverable())
                    .count(),
                partial: snapshot.partial.len(),
            });
        }
        Ok(result)
    }

    #[cfg(test)]
    pub fn cleanup(&self, now: u64) -> Result<usize, String> {
        cleanup_directory(&self.directory, now, None)
    }

    /// Sweep only directories created by this recovery format. This includes
    /// inactive workspaces without opening their sessions or old archives.
    pub fn sweep_home(home: &Path, now: u64, cancel: &Cancellation) -> Result<usize, String> {
        check_sweep(Some(cancel))?;
        let checkpoints = home.join("checkpoints");
        if !ordinary_directory(&checkpoints)? {
            return Ok(0);
        }
        let mut removed = 0;
        for entry in fs::read_dir(&checkpoints).map_err(|e| e.to_string())? {
            check_sweep(Some(cancel))?;
            let Ok(entry) = entry else { continue };
            let directory = entry.path();
            if ordinary_directory(&directory).unwrap_or(false)
                && recognized_workspace(&directory).unwrap_or(false)
            {
                let result = cleanup_directory(&directory, now, Some(cancel));
                check_sweep(Some(cancel))?;
                if let Ok(count) = result {
                    removed += count;
                }
            }
        }
        check_sweep(Some(cancel))?;
        Ok(removed)
    }
}

fn cleanup_directory(
    directory: &Path,
    now: u64,
    cancel: Option<&Cancellation>,
) -> Result<usize, String> {
    let mut removed = 0;
    for entry in fs::read_dir(directory).map_err(|e| e.to_string())? {
        check_sweep(cancel)?;
        let entry = entry.map_err(|e| e.to_string())?;
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        let pending = name.starts_with('.') && name.ends_with(".pending");
        let reference = if pending {
            name.strip_prefix('.')
                .and_then(|name| name.strip_suffix(".pending"))
        } else {
            name.strip_suffix(".rcp")
        };
        let Some(reference) = reference else {
            continue;
        };
        if !valid_reference(reference)
            || created_from_reference(reference)
                .unwrap()
                .saturating_add(TTL_MILLIS)
                > now
        {
            continue;
        }
        let path = entry.path();
        if !ordinary_file(&path)? {
            continue;
        }
        let file = match crate::disk::private_options()
            .read(true)
            .write(true)
            .open(&path)
        {
            Ok(file) => file,
            Err(_) => continue,
        };
        let Some(_lease) = locks::Exclusive::try_from_file(file).map_err(|e| e.to_string())? else {
            continue;
        };
        // The generated reference is the creation clock. Strict names and an
        // exclusive lock identify both intact packs and crashed/corrupt ones,
        // so damaged contents do not outlive the same fixed retention window.
        check_sweep(cancel)?;
        fs::remove_file(&path).map_err(|e| format!("Could not expire {reference}: {e}"))?;
        removed += 1;
    }
    if removed != 0 {
        files::sync_directory(directory).map_err(|e| e.to_string())?;
    }
    Ok(removed)
}

fn check_sweep(cancel: Option<&Cancellation>) -> Result<(), String> {
    if cancel.is_some_and(Cancellation::requested) {
        Err("Operation cancelled".into())
    } else {
        Ok(())
    }
}

struct ReadLease {
    file: File,
}

impl Drop for ReadLease {
    fn drop(&mut self) {
        // A concurrent Unix fork may retain a descriptor until exec. Release
        // the lock when our last reader ends, including failed loads.
        let _ = self.file.unlock();
    }
}

pub struct Loaded {
    pub snapshot: Snapshot,
    lease: Arc<ReadLease>,
}

impl Loaded {
    pub fn reader(&self, state: &State) -> Result<Reader, String> {
        let State::File { offset, length, .. } = state else {
            return Err("Recovery state has no file bytes".into());
        };
        Ok(Reader {
            lease: Arc::clone(&self.lease),
            start: *offset,
            length: *length,
            at: 0,
        })
    }

    pub fn verify(&self, state: &State, cancel: &Cancellation) -> Result<(), String> {
        let State::File { length, digest, .. } = state else {
            return Err("Recovery state has no file bytes".into());
        };
        let mut reader = self.reader(state)?;
        let mut hasher = hash::Hasher::new(*length);
        let mut buffer = [0u8; 64 * 1024];
        loop {
            if cancel.requested() {
                return Err("Operation cancelled".into());
            }
            let n = reader.read(&mut buffer).map_err(|e| e.to_string())?;
            if n == 0 {
                break;
            }
            hasher.update(&buffer[..n]);
        }
        if hasher.finish() != *digest {
            return Err("Corrupt recovery payload".into());
        }
        Ok(())
    }
}

pub struct Reader {
    lease: Arc<ReadLease>,
    start: u64,
    length: u64,
    at: u64,
}

impl Read for Reader {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        let n = self.length.saturating_sub(self.at).min(buffer.len() as u64) as usize;
        if n == 0 {
            return Ok(0);
        }
        let read = read_at(&self.lease.file, &mut buffer[..n], self.start + self.at)?;
        if read == 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "Truncated recovery payload",
            ));
        }
        self.at += read as u64;
        Ok(read)
    }
}

impl Seek for Reader {
    fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
        let next = match pos {
            SeekFrom::Start(n) => n as i128,
            SeekFrom::Current(n) => self.at as i128 + n as i128,
            SeekFrom::End(n) => self.length as i128 + n as i128,
        };
        if next < 0 || next > self.length as i128 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "Outside recovery payload",
            ));
        }
        self.at = next as u64;
        Ok(self.at)
    }
}

fn valid_reference(reference: &str) -> bool {
    let bytes = reference.as_bytes();
    bytes.len() == 45
        && bytes.starts_with(b"rc-")
        && bytes[19] == b'-'
        && bytes[28] == b'-'
        && bytes[3..19]
            .iter()
            .chain(&bytes[20..28])
            .chain(&bytes[29..])
            .all(u8::is_ascii_hexdigit)
}

fn created_from_reference(reference: &str) -> Option<u64> {
    valid_reference(reference)
        .then(|| u64::from_str_radix(&reference[3..19], 16).ok())
        .flatten()
}

#[cfg(unix)]
fn read_at(file: &File, buffer: &mut [u8], offset: u64) -> std::io::Result<usize> {
    use std::os::unix::fs::FileExt;
    file.read_at(buffer, offset)
}

#[cfg(windows)]
fn read_at(file: &File, buffer: &mut [u8], offset: u64) -> std::io::Result<usize> {
    use std::os::windows::fs::FileExt;
    file.seek_read(buffer, offset)
}

#[cfg(test)]
mod tests;
