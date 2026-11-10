//! File-tool serialization, independent of workspace recovery.
use crate::{cancel::Cancellation, disk};
use std::path::{Path, PathBuf};

const STRIPES: u64 = 256;

/// Serialize modifications of one file across the current account's processes.
///
/// Paths fold to lowercase and share a bounded set of lock files, so aliases
/// on case-insensitive volumes meet and the directory never grows; unrelated
/// paths sharing a stripe only wait for each other. An unavailable lock must
/// stop the modification: a full write has no previous content to recheck.
pub(super) fn acquire(
    path: &Path,
    cancellation: &Cancellation,
) -> Result<disk::lock::Exclusive, String> {
    if cancellation.requested() {
        return Err("Operation cancelled".into());
    }
    acquire_at(
        &lock_path(&lock_root(&std::env::temp_dir())?, path),
        cancellation,
    )
}

#[cfg(unix)]
fn lock_root(temp: &Path) -> Result<PathBuf, String> {
    // The temporary directory can be shared by accounts. The effective UID
    // keeps their private lock directories separate without using HOME.
    Ok(lock_root_for_uid(temp, crate::native::effective_user_id()?))
}

#[cfg(unix)]
fn lock_root_for_uid(temp: &Path, uid: u32) -> PathBuf {
    temp.join(format!("jecode-file-locks-{uid}"))
}

#[cfg(not(unix))]
fn lock_root(temp: &Path) -> Result<PathBuf, String> {
    // Windows supplies a per-account temporary directory.
    Ok(temp.join("jecode-file-locks"))
}

fn lock_path(root: &Path, path: &Path) -> PathBuf {
    let folded = path.to_string_lossy().to_lowercase();
    let stripe = disk::hash::digest(folded.as_bytes()).0[0] % STRIPES;
    root.join(format!("{stripe:02x}.lock"))
}

fn acquire_at(lock: &Path, cancellation: &Cancellation) -> Result<disk::lock::Exclusive, String> {
    if cancellation.requested() {
        return Err("Operation cancelled".into());
    }
    // Keep the empty lock file: unlinking it would let a new caller bypass
    // another process already waiting on the old file. No file contents are saved.
    let file = disk::lock::open(lock)
        .map_err(|error| format!("Could not open the file-modification lock: {error}"))?;
    disk::lock::Exclusive::from_file_cancellable(file, cancellation)
        .map_err(|error| format!("Could not lock the file for modification: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::Directory;

    #[test]
    fn unavailable_lock_refuses_to_modify() {
        let directory = Directory::new();
        let occupied = directory.path().join("occupied.lock");
        std::fs::create_dir(&occupied).unwrap();
        let error = acquire_at(&occupied, &Cancellation::default())
            .err()
            .expect("opening a directory for writing must fail");
        assert!(error.contains("Could not open the file-modification lock"));
    }

    #[test]
    fn cancelled_lock_creates_no_directory() {
        let directory = Directory::new();
        let root = directory.path().join("unused-locks");
        let cancellation = Cancellation::default();
        cancellation.cancel();
        let error = acquire_at(&root.join("00.lock"), &cancellation)
            .err()
            .expect("cancelled acquisition must fail");
        assert_eq!(error, "Operation cancelled");
        assert!(!root.exists());
    }

    #[test]
    fn lock_namespace_is_shared_by_workspace_path_within_one_account() {
        let directory = Directory::new();
        let root_a = directory.path().join("account-a");
        let root_b = directory.path().join("account-b");
        let workspace_path = directory.path().join("shared.txt");
        let first = lock_path(&root_a, &workspace_path);
        let folded_alias = PathBuf::from(workspace_path.to_string_lossy().to_uppercase());
        assert_eq!(first, lock_path(&root_a, &folded_alias));
        assert_ne!(first, lock_path(&root_b, &workspace_path));
        assert!(first.starts_with(root_a));
    }

    #[cfg(unix)]
    #[test]
    fn shared_temporary_directory_separates_effective_users() {
        let directory = Directory::new();
        assert_ne!(
            lock_root_for_uid(directory.path(), 1000),
            lock_root_for_uid(directory.path(), 1001)
        );
    }

    #[cfg(unix)]
    #[test]
    fn native_account_identity_matches_a_new_files_owner() {
        use std::os::unix::fs::MetadataExt;
        let directory = Directory::new();
        let file = directory.path().join("owner");
        std::fs::write(&file, b"fixture").unwrap();
        assert_eq!(
            crate::native::effective_user_id().unwrap(),
            std::fs::metadata(file).unwrap().uid()
        );
    }
}
