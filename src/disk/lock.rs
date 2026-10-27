//! Operating-system file locks shared by private stores.
use crate::cancel::Cancellation;
use std::fs::{File, TryLockError};
use std::path::Path;

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

pub(crate) fn lock_cancellable(file: &File, cancellation: &Cancellation) -> std::io::Result<()> {
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
