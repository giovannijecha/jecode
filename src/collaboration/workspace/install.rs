//! Exact file installation into the project: staged siblings, atomic renames
//! and removal that survives a Windows read-only attribute.
use std::fs::{self, OpenOptions};
use std::path::{Path, PathBuf};

/// A complete replacement waiting beside its destination. Dropping it without
/// installation removes the temporary file.
pub(super) struct PendingFile {
    path: PathBuf,
    temporary: PathBuf,
    installed: bool,
}

impl PendingFile {
    /// Replaces the destination with the synced staged file by an atomic
    /// rename. The directory entry itself is not synced.
    pub(super) fn install(mut self) -> std::io::Result<()> {
        #[cfg(windows)]
        let original_permissions = clear_readonly_file(&self.path)?;
        let result = fs::rename(&self.temporary, &self.path);
        #[cfg(windows)]
        if let Err(error) = result {
            return Err(restore_permissions_on_error(
                &self.path,
                original_permissions,
                error,
            ));
        }
        #[cfg(not(windows))]
        result?;
        self.installed = true;
        Ok(())
    }
}

impl Drop for PendingFile {
    fn drop(&mut self) {
        if !self.installed {
            #[cfg(windows)]
            if let Ok(metadata) = fs::metadata(&self.temporary)
                && metadata.permissions().readonly()
            {
                let mut permissions = metadata.permissions();
                #[allow(clippy::permissions_set_readonly_false)]
                permissions.set_readonly(false);
                let _ = fs::set_permissions(&self.temporary, permissions);
            }
            let _ = fs::remove_file(&self.temporary);
        }
    }
}

/// Streams a replacement and its workspace mode (Unix permission bits, or
/// nonzero for read-only elsewhere) to a synced sibling of `path`.
pub(super) fn stage(
    path: &Path,
    mut source: impl std::io::Read,
    mode: u32,
) -> std::io::Result<PendingFile> {
    let temporary = crate::disk::temporary_beside(path);
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)?;
    let pending = PendingFile {
        path: path.to_path_buf(),
        temporary,
        installed: false,
    };
    std::io::copy(&mut source, &mut file)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(fs::Permissions::from_mode(mode & 0o7777))?;
    }
    // A new file starts writable, so only read-only needs applying.
    #[cfg(not(unix))]
    if mode != 0 {
        super::files::apply_mode(&pending.temporary, mode).map_err(std::io::Error::other)?;
    }
    file.sync_all()?;
    Ok(pending)
}

/// Removes a file or link, clearing a Windows read-only attribute first.
pub(super) fn remove(path: &Path) -> std::io::Result<()> {
    let metadata = match fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
        Ok(metadata) => metadata,
    };
    if metadata.is_dir() {
        return fs::remove_dir(path);
    }
    #[cfg(windows)]
    let original_permissions = clear_readonly_file_with_metadata(path, &metadata)?;
    let result = match fs::remove_file(path) {
        #[cfg(windows)]
        Err(_) if fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink()) => {
            fs::remove_dir(path)
        }
        result => result,
    };
    match result {
        Ok(()) => Ok(()),
        #[cfg(windows)]
        Err(error) => Err(restore_permissions_on_error(
            path,
            original_permissions,
            error,
        )),
        #[cfg(not(windows))]
        Err(error) => Err(error),
    }
}

#[cfg(windows)]
fn clear_readonly_file(path: &Path) -> std::io::Result<Option<fs::Permissions>> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => clear_readonly_file_with_metadata(path, &metadata),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

#[cfg(windows)]
fn clear_readonly_file_with_metadata(
    path: &Path,
    metadata: &fs::Metadata,
) -> std::io::Result<Option<fs::Permissions>> {
    if !metadata.is_file() || !metadata.permissions().readonly() {
        return Ok(None);
    }
    let original = metadata.permissions();
    let mut writable = original.clone();
    #[allow(clippy::permissions_set_readonly_false)]
    writable.set_readonly(false);
    fs::set_permissions(path, writable)?;
    Ok(Some(original))
}

#[cfg(windows)]
fn restore_permissions_on_error(
    path: &Path,
    original: Option<fs::Permissions>,
    error: std::io::Error,
) -> std::io::Error {
    let Some(original) = original else {
        return error;
    };
    match fs::set_permissions(path, original) {
        Ok(()) => error,
        Err(rollback) => std::io::Error::new(
            error.kind(),
            format!(
                "{error}; could not restore permissions for {}: {rollback}",
                path.display()
            ),
        ),
    }
}

#[cfg(test)]
#[path = "tests/install.rs"]
mod tests;
