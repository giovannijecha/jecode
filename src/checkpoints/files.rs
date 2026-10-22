//! Private archive file helpers and exact workspace file installation.
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;
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
        crate::sessions::now(),
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
        // The root is canonical and ordinary directory entries cannot redirect
        // traversal. Resolve only Windows reparse directories; avoid reopening
        // every ordinary ancestor once for each file in a full capture.
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            const REPARSE_POINT: u32 = 0x400;
            if metadata.file_attributes() & REPARSE_POINT != 0 {
                let resolved = fs::canonicalize(&parent)
                    .map_err(|error| format!("Could not resolve parent of {relative}: {error}"))?;
                if !resolved.starts_with(root) {
                    return Err(format!("{relative} has a parent outside the workspace"));
                }
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
    replace(path, bytes, true)
}

/// Replaces a private archive file whole without waiting for the disk; a
/// crash can leave the previous content.
pub fn replace_unsynced(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    replace(path, bytes, false)
}

fn replace(path: &Path, bytes: &[u8], sync: bool) -> std::io::Result<()> {
    let parent = path.parent().unwrap_or(Path::new("."));
    private_directory(parent)?;
    let temporary = parent.join(format!(".{}.tmp", unique("write")));
    let result = (|| {
        let mut file = options().write(true).create_new(true).open(&temporary)?;
        file.write_all(bytes)?;
        if sync {
            file.sync_all()?;
        }
        drop(file);
        fs::rename(&temporary, path)?;
        if sync {
            sync_directory(parent)?;
        }
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

/// Installs exact bytes with a temporary sibling and an atomic rename. A
/// read-only target is replaced too, because recovery restores attributes.
pub fn install(path: &Path, bytes: &[u8], mode: Mode) -> std::io::Result<()> {
    let parent = path.parent().unwrap_or(Path::new("."));
    let temporary = parent.join(format!(
        ".jecode-{}-{}.tmp",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        #[cfg(unix)]
        mode.apply(&file)?;
        drop(file);
        if let Ok(metadata) = fs::symlink_metadata(path)
            && metadata.is_file()
            && metadata.permissions().readonly()
        {
            let mut permissions = metadata.permissions();
            #[allow(clippy::permissions_set_readonly_false)]
            permissions.set_readonly(false);
            fs::set_permissions(path, permissions)?;
        }
        fs::rename(&temporary, path)?;
        #[cfg(not(unix))]
        if mode.0 & Mode::READONLY != 0 {
            mode.apply_path(path)?;
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

/// Removes a file or link, clearing a Windows read-only attribute first.
pub fn remove(path: &Path) -> std::io::Result<()> {
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
        Ok(metadata) if metadata.is_dir() => return fs::remove_dir(path),
        Ok(metadata) if !metadata.file_type().is_symlink() && metadata.permissions().readonly() => {
            let mut permissions = metadata.permissions();
            #[allow(clippy::permissions_set_readonly_false)]
            permissions.set_readonly(false);
            fs::set_permissions(path, permissions)?;
        }
        Ok(_) => {}
    }
    match fs::remove_file(path) {
        #[cfg(windows)]
        Err(_) if fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink()) => {
            fs::remove_dir(path)
        }
        result => result,
    }
}

pub fn symlink(target: &str, path: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(target, path)
    }
    #[cfg(windows)]
    {
        let resolved = path.parent().unwrap_or(Path::new(".")).join(target);
        if resolved.is_dir() {
            std::os::windows::fs::symlink_dir(target, path)
        } else {
            std::os::windows::fs::symlink_file(target, path)
        }
    }
}
