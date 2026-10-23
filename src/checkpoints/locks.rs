//! Cross-process coordination through operating-system file locks, released
//! automatically when a process exits.
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
        Self::from_file(open(path)?)
    }

    pub fn from_file(file: File) -> std::io::Result<Self> {
        file.lock()?;
        Ok(Self(file))
    }

    pub fn file_mut(&mut self) -> &mut File {
        &mut self.0
    }

    pub fn try_acquire(path: &Path) -> std::io::Result<Option<Self>> {
        Self::try_from_file(open(path)?)
    }

    pub fn try_from_file(file: File) -> std::io::Result<Option<Self>> {
        match file.try_lock() {
            Ok(()) => Ok(Some(Self(file))),
            Err(std::fs::TryLockError::WouldBlock) => Ok(None),
            Err(std::fs::TryLockError::Error(error)) => Err(error),
        }
    }

    pub fn acquire_cancellable(
        path: &Path,
        cancellation: &crate::cancel::Cancellation,
    ) -> std::io::Result<Self> {
        loop {
            if cancellation.requested() {
                return Err(std::io::Error::other("Operation cancelled"));
            }
            if let Some(lock) = Self::try_acquire(path)? {
                return Ok(lock);
            }
            cancellation
                .wait(std::time::Duration::from_millis(10))
                .map_err(std::io::Error::other)?;
        }
    }
}

impl Drop for Exclusive {
    fn drop(&mut self) {
        // A concurrent Unix fork can retain a duplicate descriptor until exec.
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

    /// Acquire path stripes in ascending order to avoid multi-path deadlocks.
    pub fn acquire_cancellable(
        self: &Arc<Self>,
        paths: &[&str],
        cancellation: &crate::cancel::Cancellation,
    ) -> std::io::Result<Lease> {
        let mut stripes: Vec<usize> = paths.iter().map(|path| Self::stripe(path)).collect();
        stripes.sort_unstable();
        stripes.dedup();
        self.stripes_cancellable(stripes, Some(cancellation))
    }

    pub fn acquire_all_cancellable(
        self: &Arc<Self>,
        cancellation: &crate::cancel::Cancellation,
    ) -> std::io::Result<Lease> {
        self.stripes_cancellable((0..STRIPES).collect(), Some(cancellation))
    }

    fn stripes_cancellable(
        self: &Arc<Self>,
        stripes: Vec<usize>,
        cancellation: Option<&crate::cancel::Cancellation>,
    ) -> std::io::Result<Lease> {
        let mut lease = Lease {
            leases: self.clone(),
            stripes: Vec::new(),
        };
        for stripe in stripes {
            {
                let mut held = self.held.lock().unwrap_or_else(|e| e.into_inner());
                while held.contains(&stripe) {
                    if cancellation.is_some_and(|token| token.requested()) {
                        return Err(std::io::Error::other("Operation cancelled"));
                    }
                    held = self
                        .released
                        .wait_timeout(held, std::time::Duration::from_millis(10))
                        .unwrap_or_else(|e| e.into_inner())
                        .0;
                }
                held.insert(stripe);
            }
            let path = self.directory.join(format!("{stripe:02x}.lock"));
            let acquired = match cancellation {
                Some(token) => Exclusive::acquire_cancellable(&path, token),
                None => Exclusive::acquire(&path),
            };
            match acquired {
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
