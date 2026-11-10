//! File-tool serialization, independent of workspace recovery.
use crate::{cancel::Cancellation, disk};
use std::path::Path;

const STRIPES: u64 = 256;

/// Serialize modifications of one file across threads and processes.
///
/// Paths fold to lowercase and share a bounded set of lock files, so aliases
/// on case-insensitive volumes meet and the directory never grows; unrelated
/// paths sharing a stripe only wait for each other. The lock is advisory: when
/// its file cannot be opened, as in a temporary directory owned by another
/// user, the modification proceeds and the changed-file check before each
/// replacement still rejects a stale edit.
pub(super) fn acquire(
    path: &Path,
    cancellation: &Cancellation,
) -> Result<Option<disk::lock::Exclusive>, String> {
    let folded = path.to_string_lossy().to_lowercase();
    let stripe = disk::hash::digest(folded.as_bytes()).0[0] % STRIPES;
    let lock = std::env::temp_dir()
        .join("jecode-file-locks")
        .join(format!("{stripe:02x}.lock"));
    // Keep the empty lock file: unlinking it would let a new caller bypass
    // another process already waiting on the old file. No file contents are saved.
    let Ok(file) = disk::lock::open(&lock) else {
        return Ok(None);
    };
    disk::lock::Exclusive::from_file_cancellable(file, cancellation)
        .map(Some)
        .map_err(|error| format!("Could not lock the file for modification: {error}"))
}
