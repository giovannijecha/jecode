//! Cross-process coordination through operating-system file locks, released
//! automatically when a process exits.
use crate::cancel::Cancellation;
use std::collections::HashSet;
use std::fs::File;
use std::path::PathBuf;
use std::sync::{Arc, Condvar, Mutex, OnceLock};

const STRIPES: usize = 256;

use crate::disk::lock::lock_cancellable;
pub use crate::disk::lock::{Exclusive, open};

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
