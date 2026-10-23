//! Cross-process coordination through operating-system file locks, released
//! automatically when a process exits.
use crate::cancel::Cancellation;
use std::collections::HashSet;
use std::fs::{File, TryLockError};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex, OnceLock};

const STRIPES: usize = 256;

pub fn open(path: &Path) -> std::io::Result<File> {
    if let Some(parent) = path.parent() {
        crate::disk::private_directory(parent)?;
    }
    crate::disk::private_options()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)
}

fn lock_cancellable(file: &File, cancellation: &Cancellation) -> std::io::Result<()> {
    loop {
        if cancellation.requested() {
            return Err(std::io::Error::other("Operation cancelled"));
        }
        match file.try_lock() {
            Ok(()) => return Ok(()),
            Err(TryLockError::WouldBlock) => cancellation
                .wait(std::time::Duration::from_millis(10))
                .map_err(std::io::Error::other)?,
            Err(TryLockError::Error(error)) => return Err(error),
        }
    }
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

    pub fn try_from_file(file: File) -> std::io::Result<Option<Self>> {
        match file.try_lock() {
            Ok(()) => Ok(Some(Self(file))),
            Err(TryLockError::WouldBlock) => Ok(None),
            Err(TryLockError::Error(error)) => Err(error),
        }
    }

    pub fn acquire_cancellable(path: &Path, cancellation: &Cancellation) -> std::io::Result<Self> {
        let file = open(path)?;
        lock_cancellable(&file, cancellation)?;
        Ok(Self(file))
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
/// its own threads before taking the cross-process lock. Stripe handles stay
/// open with the store, so a whole-workspace lease only locks and unlocks.
pub struct Leases {
    directory: PathBuf,
    handles: Vec<OnceLock<File>>,
    held: Mutex<HashSet<usize>>,
    released: Condvar,
}

pub struct Lease {
    leases: Arc<Leases>,
    stripes: Vec<usize>,
}

impl Leases {
    pub fn new(directory: PathBuf) -> Arc<Self> {
        Arc::new(Self {
            directory,
            handles: (0..STRIPES).map(|_| OnceLock::new()).collect(),
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
        cancellation: &Cancellation,
    ) -> std::io::Result<Lease> {
        let mut stripes: Vec<usize> = paths.iter().map(|path| Self::stripe(path)).collect();
        stripes.sort_unstable();
        stripes.dedup();
        self.stripes_cancellable(stripes, cancellation)
    }

    pub fn acquire_all_cancellable(
        self: &Arc<Self>,
        cancellation: &Cancellation,
    ) -> std::io::Result<Lease> {
        self.stripes_cancellable((0..STRIPES).collect(), cancellation)
    }

    fn stripes_cancellable(
        self: &Arc<Self>,
        stripes: Vec<usize>,
        cancellation: &Cancellation,
    ) -> std::io::Result<Lease> {
        let mut lease = Lease {
            leases: self.clone(),
            stripes: Vec::with_capacity(stripes.len()),
        };
        for stripe in stripes {
            {
                let mut held = self.held.lock().unwrap_or_else(|e| e.into_inner());
                while held.contains(&stripe) {
                    if cancellation.requested() {
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
            match self.lock(stripe, cancellation) {
                Ok(()) => lease.stripes.push(stripe),
                Err(error) => {
                    self.release(stripe);
                    return Err(error);
                }
            }
        }
        Ok(lease)
    }

    /// Takes the cross-process lock of a stripe this thread already holds.
    fn lock(&self, stripe: usize, cancellation: &Cancellation) -> std::io::Result<()> {
        let slot = &self.handles[stripe];
        let file = match slot.get() {
            Some(file) => file,
            None => {
                let file = open(&self.directory.join(format!("{stripe:02x}.lock")))?;
                slot.get_or_init(|| file)
            }
        };
        lock_cancellable(file, cancellation)
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
        for stripe in self.stripes.drain(..).rev() {
            // A concurrent Unix fork can retain a duplicate descriptor until exec.
            if let Some(file) = self.leases.handles[stripe].get() {
                let _ = file.unlock();
            }
            self.leases.release(stripe);
        }
    }
}
