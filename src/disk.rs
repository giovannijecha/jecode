//! File system helpers shared by Jecode's private stores.
use std::fs::{self, Metadata, OpenOptions};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_TEMPORARY: AtomicU64 = AtomicU64::new(0);

/// A fresh hidden name beside `path` for staging its replacement. One
/// process-wide counter keeps concurrent writers in a directory apart.
pub fn temporary_beside(path: &Path) -> PathBuf {
    path.with_file_name(format!(
        ".jecode-{}-{}.tmp",
        std::process::id(),
        NEXT_TEMPORARY.fetch_add(1, Ordering::Relaxed)
    ))
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
