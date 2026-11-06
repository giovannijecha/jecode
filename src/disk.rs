//! File system helpers shared by Jecode's private stores.
use std::fs::{self, Metadata, OpenOptions};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

pub mod hash;
pub mod lock;
pub mod owned;

static NEXT_TEMPORARY: AtomicU64 = AtomicU64::new(0);

/// Restricts a private file or directory to its owner. The ACL change runs in
/// the native Windows helper that also serves account storage.
#[cfg(windows)]
pub fn protect_owner_only(path: &Path, directory: bool) -> Result<(), String> {
    crate::auth::openai::protect(path, directory)
}

/// A fresh hidden name beside `path` for staging its replacement. One
/// process-wide counter keeps concurrent writers in a directory apart.
pub fn temporary_beside(path: &Path) -> PathBuf {
    path.with_file_name(format!(
        ".jecode-{}-{}.tmp",
        std::process::id(),
        NEXT_TEMPORARY.fetch_add(1, Ordering::Relaxed)
    ))
}

/// Whether a file name has the form `temporary_beside` creates, so crash
/// recovery can recognize an abandoned staging file.
pub fn temporary_name(name: &str) -> bool {
    let Some(parts) = name
        .strip_prefix(".jecode-")
        .and_then(|name| name.strip_suffix(".tmp"))
    else {
        return false;
    };
    let mut parts = parts.split('-');
    parts.next().is_some_and(|part| part.parse::<u32>().is_ok())
        && parts.next().is_some_and(|part| part.parse::<u64>().is_ok())
        && parts.next().is_none()
}

/// Options for files only the current user may read: mode 0600 on Unix.
pub fn private_options() -> OpenOptions {
    #[allow(unused_mut)]
    let mut options = OpenOptions::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options
}

/// Create a missing directory only the current user may enter: mode 0700 on
/// Unix. An existing directory keeps its permissions.
pub fn private_directory(path: &Path) -> std::io::Result<()> {
    if !path.is_dir() {
        fs::create_dir_all(path)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
        }
    }
    Ok(())
}

/// A symbolic link or, on Windows, any reparse point such as a junction.
pub fn linked(metadata: &Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        metadata.file_attributes() & 0x400 != 0 // FILE_ATTRIBUTE_REPARSE_POINT
    }
    #[cfg(not(windows))]
    {
        metadata.file_type().is_symlink()
    }
}

/// Inspect an entry without following links, distinguishing absence from an
/// inspection failure. Callers decide whether links are rejected or skipped.
pub fn metadata_if_present(path: &Path) -> std::io::Result<Option<Metadata>> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => Ok(Some(metadata)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

pub fn ordinary_directory(metadata: &Metadata) -> bool {
    metadata.is_dir() && !linked(metadata)
}

/// Create a private directory if absent, refusing links and checking the result.
/// Returns whether it was created so durable stores can sync its parent.
pub fn ensure_private_directory(path: &Path) -> std::io::Result<bool> {
    let existing = metadata_if_present(path)?;
    if let Some(metadata) = existing {
        if ordinary_directory(&metadata) {
            return Ok(false);
        }
    } else {
        private_directory(path)?;
        if ordinary_directory(&fs::symlink_metadata(path)?) {
            return Ok(true);
        }
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::InvalidInput,
        "Path is a link, junction or non-directory entry",
    ))
}

/// Replace a private file with a synced staging file in the same directory.
pub fn atomic_write(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let parent = path.parent().unwrap_or(Path::new("."));
    private_directory(parent)?;
    let temporary = temporary_beside(path);
    let result = (|| {
        let mut file = private_options()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temporary, path)?;
        sync_directory(parent)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

/// Windows does not expose directory flushing through the standard library.
pub fn sync_directory(_path: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        fs::File::open(_path)?.sync_all()
    }
    #[cfg(not(unix))]
    {
        Ok(())
    }
}

/// Remove a known directory prefix without normalizing the remaining path.
/// Unix volumes may differ in case and Unicode normalization rules: a different
/// spelling is an alias only when the filesystem identifies the same directory.
pub fn strip_prefix<'a>(path: &'a Path, root: &Path) -> Option<&'a Path> {
    let mut parts = path.components();
    let mut same = true;
    for expected in root.components() {
        let actual = parts.next()?;
        same &= if cfg!(windows) {
            actual.as_os_str().to_string_lossy().to_lowercase()
                == expected.as_os_str().to_string_lossy().to_lowercase()
        } else {
            actual == expected
        };
    }
    if !same {
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            let prefix: PathBuf = path.components().take(root.components().count()).collect();
            let actual = fs::metadata(prefix).ok()?;
            let expected = fs::metadata(root).ok()?;
            if !actual.is_dir()
                || !expected.is_dir()
                || actual.dev() != expected.dev()
                || actual.ino() != expected.ino()
            {
                return None;
            }
        }
        #[cfg(not(unix))]
        return None;
    }
    Some(parts.as_path())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::Directory;

    #[test]
    fn directory_aliases_follow_the_filesystems_case_rules() {
        let temp = Directory::new();
        let upper = temp.path().join("Project");
        let lower = temp.path().join("project");
        fs::create_dir(&upper).unwrap();
        let distinct = match fs::create_dir(&lower) {
            Ok(()) => true,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => false,
            Err(error) => panic!("{error}"),
        };
        let path = lower.join("missing/../child");
        assert_eq!(
            strip_prefix(&path, &upper),
            (!distinct).then_some(Path::new("missing/../child"))
        );
        assert_eq!(
            strip_prefix(&upper.join("child"), &upper),
            Some(Path::new("child"))
        );
    }

    #[cfg(unix)]
    #[test]
    fn linked_directory_aliases_preserve_the_unresolved_suffix() {
        let temp = Directory::new();
        let root = temp.path().join("root");
        let alias = temp.path().join("alias");
        fs::create_dir(&root).unwrap();
        std::os::unix::fs::symlink(&root, &alias).unwrap();
        assert_eq!(
            strip_prefix(&alias.join("link/../child"), &root),
            Some(Path::new("link/../child"))
        );
    }
}
