//! Private archive file helpers and exact workspace file installation.
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);

pub fn options() -> OpenOptions {
    #[allow(unused_mut)]
    let mut options = OpenOptions::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options
}

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

pub fn unique(prefix: &str) -> String {
    format!(
        "{prefix}-{}-{}-{}",
        std::process::id(),
        super::clock::now(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    )
}

/// Rejects a workspace path whose existing parent traverses a link, junction,
/// or another root. A regular file parent makes descendants absent, not unsafe.
/// The final entry may itself be a link.
pub fn confined_parent(root: &Path, relative: &str) -> Result<(), String> {
    if !relative.split('/').all(normal_component) {
        return Err(format!("Invalid workspace path {relative:?}"));
    }
    let mut parent = root.to_path_buf();
    let mut parts = relative.split('/').peekable();
    while let Some(part) = parts.next() {
        if parts.peek().is_none() {
            break;
        }
        parent.push(part);
        let metadata = match fs::symlink_metadata(&parent) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => break,
            Err(error) => return Err(format!("Could not inspect parent of {relative}: {error}")),
        };
        if metadata.file_type().is_symlink() {
            return Err(format!("{relative} has a linked parent"));
        }
        if !metadata.is_dir() {
            break;
        }
        // A reparse directory may redirect traversal or expose opaque contents;
        // never infer the state of descendants through it.
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            const REPARSE_POINT: u32 = 0x400;
            if metadata.file_attributes() & REPARSE_POINT != 0 {
                return Err(format!("{relative} has a reparse parent"));
            }
        }
    }
    Ok(())
}

/// Every joined component must be a filename, including on Windows where an
/// embedded drive prefix can replace a PathBuf rather than append to it.
pub fn normal_component(part: &str) -> bool {
    let mut components = Path::new(part).components();
    let normal = matches!(components.next(), Some(std::path::Component::Normal(_)))
        && components.next().is_none();
    #[cfg(windows)]
    let normal = normal && !part.contains(':');
    normal
}

/// Replaces a private archive file through a synced temporary file.
pub fn atomic_write(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let parent = path.parent().unwrap_or(Path::new("."));
    private_directory(parent)?;
    let temporary = parent.join(format!(".{}.tmp", unique("write")));
    let result = (|| {
        let mut file = options().write(true).create_new(true).open(&temporary)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temporary, path)?;
        sync_directory(parent)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

/// Persists a directory entry change where the platform exposes it through std.
/// Windows does not expose directory flushing through std.
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

/// Exact workspace file metadata that recovery can reproduce.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Mode(pub u32);

impl Mode {
    #[cfg(not(unix))]
    pub const READONLY: u32 = 1 << 31;

    pub fn of(metadata: &fs::Metadata) -> Self {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            Self(metadata.permissions().mode() & 0o7777)
        }
        #[cfg(not(unix))]
        {
            Self(if metadata.permissions().readonly() {
                Self::READONLY
            } else {
                0
            })
        }
    }

    /// Sets the mode on a new file before it is renamed into place.
    #[cfg(unix)]
    pub fn apply(self, file: &fs::File) -> std::io::Result<()> {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(fs::Permissions::from_mode(self.0 & 0o7777))
    }

    #[cfg(any(test, not(unix)))]
    pub fn apply_path(self, path: &Path) -> std::io::Result<()> {
        let mut permissions = fs::symlink_metadata(path)?.permissions();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            permissions.set_mode(self.0 & 0o7777);
        }
        #[cfg(not(unix))]
        permissions.set_readonly(self.0 & Self::READONLY != 0);
        fs::set_permissions(path, permissions)
    }
}

/// Changes a directory's Unix mode and syncs its inode. A directory without
/// read access needs a brief owner-accessible mode so it can be opened first;
/// the open handle remains usable after the final mode becomes restrictive.
#[cfg(unix)]
pub fn directory_mode(path: &Path, mode: Mode) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;

    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_dir() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "directory mode requires a directory",
        ));
    }
    let original = metadata.permissions();
    let (directory, elevated) = match fs::File::open(path) {
        Ok(directory) => (directory, false),
        Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => {
            let mut accessible = original.clone();
            accessible.set_mode((original.mode() & 0o7777) | 0o700);
            fs::set_permissions(path, accessible)?;
            match fs::File::open(path) {
                Ok(directory) => (directory, true),
                Err(error) => {
                    return Err(permission_rollback_error(
                        error,
                        fs::set_permissions(path, original),
                    ));
                }
            }
        }
        Err(error) => return Err(error),
    };
    if let Err(error) = mode.apply(&directory) {
        return Err(if elevated {
            permission_rollback_error(
                error,
                directory
                    .set_permissions(original)
                    .and_then(|()| directory.sync_all()),
            )
        } else {
            error
        });
    }
    directory.sync_all()
}

#[cfg(unix)]
fn permission_rollback_error(
    error: std::io::Error,
    rollback: std::io::Result<()>,
) -> std::io::Error {
    match rollback {
        Ok(()) => error,
        Err(rollback) => std::io::Error::new(
            error.kind(),
            format!("{error}; could not restore directory permissions: {rollback}"),
        ),
    }
}

/// A complete replacement waiting beside its destination. Dropping it without
/// installation removes the temporary file.
pub struct PendingFile {
    path: PathBuf,
    temporary: PathBuf,
    installed: bool,
}

/// A link created beside its destination. Creation failure never touches the
/// live entry; dropping an uninstalled link removes only the staged sibling.
pub struct PendingLink {
    path: PathBuf,
    temporary: PathBuf,
    directory: bool,
    installed: bool,
}

impl PendingLink {
    pub fn install_deferred(mut self) -> std::io::Result<()> {
        fs::rename(&self.temporary, &self.path)?;
        self.installed = true;
        Ok(())
    }
}

impl Drop for PendingLink {
    fn drop(&mut self) {
        if !self.installed {
            if self.directory {
                let _ = fs::remove_dir(&self.temporary);
            } else {
                let _ = fs::remove_file(&self.temporary);
            }
        }
    }
}

impl PendingFile {
    /// Replaces the destination after all content, metadata, and file syncing
    /// have completed. The directory entry is synced on Unix after the rename.
    #[cfg(test)]
    pub fn install(self) -> std::io::Result<()> {
        let path = self.path.clone();
        self.install_deferred()?;
        sync_parent(&path)
    }

    /// Installs the staged file; the caller must sync its parent before
    /// completing the durable operation. Restores batch shared parents.
    pub fn install_deferred(mut self) -> std::io::Result<()> {
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

/// Writes and syncs exact file bytes and mode in a temporary sibling.
#[cfg(test)]
pub fn stage(path: &Path, bytes: &[u8], mode: Mode) -> std::io::Result<PendingFile> {
    stage_reader(path, &mut std::io::Cursor::new(bytes), mode)
}

/// Streams a replacement to its sibling without retaining its contents in RAM.
pub fn stage_reader(
    path: &Path,
    mut source: impl std::io::Read,
    mode: Mode,
) -> std::io::Result<PendingFile> {
    let parent = path.parent().unwrap_or(Path::new("."));
    let temporary = parent.join(format!(
        ".jecode-{}-{}.tmp",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)?;
    let pending = PendingFile {
        path: path.to_path_buf(),
        temporary,
        installed: false,
    };
    {
        let mut file = file;
        std::io::copy(&mut source, &mut file)?;
        #[cfg(unix)]
        mode.apply(&file)?;
        #[cfg(not(unix))]
        mode.apply_path(&pending.temporary)?;
        file.sync_all()?;
        drop(file);
    }
    Ok(pending)
}

/// Installs exact bytes with a staged sibling and an atomic rename.
#[cfg(test)]
pub fn install(path: &Path, bytes: &[u8], mode: Mode) -> std::io::Result<()> {
    stage(path, bytes, mode)?.install()
}

/// Removes a file or link, clearing a Windows read-only attribute first.
pub fn remove(path: &Path) -> std::io::Result<()> {
    let metadata = match fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
        Ok(metadata) => metadata,
    };
    if metadata.is_dir() {
        fs::remove_dir(path)?;
        return sync_parent(path);
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
        Ok(()) => sync_parent(path),
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

pub fn stage_link(target: &str, directory: bool, path: &Path) -> std::io::Result<PendingLink> {
    let parent = path.parent().unwrap_or(Path::new("."));
    let temporary = parent.join(format!(".jecode-{}.tmp", unique("link")));
    let pending = PendingLink {
        path: path.to_path_buf(),
        temporary,
        directory,
        installed: false,
    };
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(target, &pending.temporary)?;
    }
    #[cfg(windows)]
    {
        if directory {
            std::os::windows::fs::symlink_dir(target, &pending.temporary)?;
        } else {
            std::os::windows::fs::symlink_file(target, &pending.temporary)?;
        }
    }
    sync_parent(&pending.temporary)?;
    Ok(pending)
}

fn sync_parent(path: &Path) -> std::io::Result<()> {
    sync_directory(path.parent().unwrap_or(Path::new(".")))
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
#[path = "files/tests.rs"]
mod tests;
