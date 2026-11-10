//! File-tool serialization, independent of workspace recovery.
use crate::{cancel::Cancellation, disk};
use std::path::Path;

pub(super) fn acquire(
    path: &Path,
    cancellation: &Cancellation,
) -> Result<disk::lock::Exclusive, String> {
    let digest = disk::hash::digest(path.as_os_str().as_encoded_bytes());
    let lock = std::env::temp_dir()
        .join("jecode-file-locks")
        .join(format!("{:016x}{:016x}.lock", digest.0[0], digest.0[1]));
    // Keep the empty lock file: unlinking it would let a new caller bypass
    // another process already waiting on the old file. No file contents are saved.
    disk::lock::Exclusive::acquire_cancellable(&lock, cancellation)
        .map_err(|error| format!("Could not lock the file for modification: {error}"))
}
