//! Cross-process coordination through operating-system file locks, released
//! automatically when a process exits. Unique owner locks are removed on
//! orderly exit; recovery removes files left by crashed processes.
use super::files;
use std::collections::HashSet;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex};

const STRIPES: usize = 256;

pub fn open(path: &Path) -> std::io::Result<File> {
    if let Some(parent) = path.parent() {
        files::private_directory(parent)?;
    }
    files::options()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)
}

/// An exclusive lock held until drop.
pub struct Exclusive(File);

impl Exclusive {
    pub fn acquire(path: &Path) -> std::io::Result<Self> {
        let file = open(path)?;
        file.lock()?;
        Ok(Self(file))
    }

    pub fn try_acquire(path: &Path) -> std::io::Result<Option<Self>> {
        let file = open(path)?;
        match file.try_lock() {
            Ok(()) => Ok(Some(Self(file))),
            Err(std::fs::TryLockError::WouldBlock) => Ok(None),
            Err(std::fs::TryLockError::Error(error)) => Err(error),
        }
    }
}

impl Drop for Exclusive {
    fn drop(&mut self) {
        // A concurrent Unix fork can retain a duplicate descriptor until exec.
        let _ = self.0.unlock();
    }
}

/// One store's unique lock file, removed after its handle closes.
pub struct Owner {
    path: PathBuf,
    held: Option<Exclusive>,
}

impl Owner {
    pub fn acquire(path: PathBuf) -> std::io::Result<Self> {
        Ok(Self {
            held: Some(Exclusive::acquire(&path)?),
            path,
        })
    }
}

impl Drop for Owner {
    fn drop(&mut self) {
        // Windows cannot remove an open lock file. A crash skips this drop;
        // the next workspace recovery still cleans that stale file.
        drop(self.held.take());
        let _ = std::fs::remove_file(&self.path);
    }
}

/// A shared lock held until drop.
pub struct Shared(File);

impl Shared {
    pub fn acquire(path: &Path) -> std::io::Result<Self> {
        let file = open(path)?;
        file.lock_shared()?;
        Ok(Self(file))
    }
}

impl Drop for Shared {
    fn drop(&mut self) {
        let _ = self.0.unlock();
    }
}

/// Striped path leases. Paths are folded to lowercase so aliases on
/// case-insensitive volumes share a stripe; unrelated paths sharing a stripe
/// only serialize. File locks are per handle, so one process also coordinates
/// its own threads before taking the cross-process lock.
pub struct Leases {
    directory: PathBuf,
    held: Mutex<HashSet<usize>>,
    released: Condvar,
}

pub struct Lease {
    leases: Arc<Leases>,
    stripes: Vec<(usize, Exclusive)>,
}

impl Leases {
    pub fn new(directory: PathBuf) -> Arc<Self> {
        Arc::new(Self {
            directory,
            held: Mutex::new(HashSet::new()),
            released: Condvar::new(),
        })
    }

    pub fn stripe(path: &str) -> usize {
        let folded = path.to_lowercase();
        (super::hash::digest(folded.as_bytes()).0[0] as usize) % STRIPES
    }

    /// Acquires every path's stripe in ascending order, so concurrent
    /// multi-path holders cannot deadlock.
    pub fn acquire(self: &Arc<Self>, paths: &[&str]) -> std::io::Result<Lease> {
        let mut stripes: Vec<usize> = paths.iter().map(|path| Self::stripe(path)).collect();
        stripes.sort_unstable();
        stripes.dedup();
        self.stripes(stripes)
    }

    /// Every stripe, for whole-scope operations.
    pub fn acquire_all(self: &Arc<Self>) -> std::io::Result<Lease> {
        self.stripes((0..STRIPES).collect())
    }

    fn stripes(self: &Arc<Self>, stripes: Vec<usize>) -> std::io::Result<Lease> {
        let mut lease = Lease {
            leases: self.clone(),
            stripes: Vec::new(),
        };
        for stripe in stripes {
            {
                let mut held = self.held.lock().unwrap_or_else(|e| e.into_inner());
                while held.contains(&stripe) {
                    held = self.released.wait(held).unwrap_or_else(|e| e.into_inner());
                }
                held.insert(stripe);
            }
            match Exclusive::acquire(&self.directory.join(format!("{stripe:02x}.lock"))) {
                Ok(lock) => lease.stripes.push((stripe, lock)),
                Err(error) => {
                    self.release(stripe);
                    return Err(error);
                }
            }
        }
        Ok(lease)
    }

    fn release(&self, stripe: usize) {
        self.held
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&stripe);
        self.released.notify_all();
    }
}

impl Drop for Lease {
    fn drop(&mut self) {
        for (stripe, lock) in self.stripes.drain(..).rev() {
            drop(lock);
            self.leases.release(stripe);
        }
    }
}

/// Whether the process that took `owners/<owner>.lock` has exited.
pub fn owner_gone(directory: &Path, owner: &str) -> bool {
    let path = super::layout::owner_lock(directory, owner);
    if !path.exists() {
        return true;
    }
    matches!(Exclusive::try_acquire(&path), Ok(Some(_)))
}
