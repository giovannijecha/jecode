use crate::disk::{self, hash};
use crate::json::Value;
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Fingerprint {
    Absent,
    File {
        size: u64,
        digest: String,
        mode: u32,
    },
    Link {
        target: String,
        directory: bool,
    },
    Directory {
        mode: u32,
    },
}

impl Fingerprint {
    pub(super) fn exists(&self) -> bool {
        !matches!(self, Self::Absent)
    }
    pub(super) fn label(&self) -> &'static str {
        match self {
            Self::Absent => "absent",
            Self::File { .. } => "file",
            Self::Link { .. } => "link",
            Self::Directory { .. } => "directory",
        }
    }
    pub(super) fn value(&self) -> Value {
        match self {
            Self::Absent => Value::object([("kind", Value::string("absent"))]),
            Self::File { size, digest, mode } => Value::object([
                ("kind", Value::string("file")),
                ("size", Value::number(size)),
                ("digest", Value::string(digest)),
                ("mode", Value::number(mode)),
            ]),
            Self::Link { target, directory } => Value::object([
                ("kind", Value::string("link")),
                ("target", Value::string(target)),
                ("directory", Value::Bool(*directory)),
            ]),
            Self::Directory { mode } => Value::object([
                ("kind", Value::string("directory")),
                ("mode", Value::number(mode)),
            ]),
        }
    }
    pub(super) fn from_value(value: &Value) -> Result<Self, String> {
        match value.get("kind").and_then(Value::as_str) {
            Some("absent") => Ok(Self::Absent),
            Some("directory") => Ok(Self::Directory {
                // Older saved baselines had no directory mode. Recovery fills
                // this from the owned copy in Workspace::from_value.
                mode: value.get("mode").and_then(Value::as_usize).unwrap_or(0) as u32,
            }),
            Some("file") => Ok(Self::File {
                size: value
                    .get("size")
                    .and_then(Value::as_usize)
                    .ok_or("Invalid file size")? as u64,
                digest: value
                    .get("digest")
                    .and_then(Value::as_str)
                    .ok_or("Invalid file digest")?
                    .into(),
                mode: value
                    .get("mode")
                    .and_then(Value::as_usize)
                    .ok_or("Invalid file mode")? as u32,
            }),
            Some("link") => Ok(Self::Link {
                target: value
                    .get("target")
                    .and_then(Value::as_str)
                    .ok_or("Invalid link target")?
                    .into(),
                directory: match value.get("directory") {
                    Some(Value::Bool(value)) => *value,
                    _ => return Err("Invalid link type".into()),
                },
            }),
            _ => Err("Invalid workspace fingerprint".into()),
        }
    }
}

pub(super) fn validate_name(name: &str) -> Result<(), String> {
    if name.is_empty()
        || name.split('/').any(|part| {
            part.is_empty()
                || part == "."
                || part == ".."
                || part.contains('\\')
                || part.contains(':')
        })
        || name == ".git"
        || name.starts_with(".git/")
    {
        return Err(format!("Invalid workspace path {name:?}"));
    }
    Ok(())
}

pub(super) fn path(root: &Path, name: &str) -> Result<PathBuf, String> {
    validate_name(name)?;
    let mut result = root.to_path_buf();
    for part in name.split('/') {
        result.push(part);
    }
    Ok(result)
}

fn parent(root: &Path, name: &str) -> Result<(), String> {
    let mut result = root.to_path_buf();
    let mut parts = name.split('/').peekable();
    while let Some(part) = parts.next() {
        if parts.peek().is_none() {
            break;
        }
        result.push(part);
        match disk::metadata_if_present(&result).map_err(|error| error.to_string())? {
            Some(metadata) if disk::ordinary_directory(&metadata) => {}
            Some(_) => return Err(format!("{name} has a linked or non-directory parent")),
            None => return Ok(()),
        }
    }
    Ok(())
}

pub(super) fn observe(root: &Path, name: &str) -> Result<Fingerprint, String> {
    parent(root, name)?;
    let path = path(root, name)?;
    let Some(metadata) = disk::metadata_if_present(&path)
        .map_err(|error| format!("Could not inspect {name}: {error}"))?
    else {
        return Ok(Fingerprint::Absent);
    };
    let kind = metadata.file_type();
    if kind.is_symlink() {
        let target =
            fs::read_link(&path).map_err(|error| format!("Could not read link {name}: {error}"))?;
        let target = target
            .to_str()
            .ok_or_else(|| format!("Unsupported link target {name}"))?;
        #[cfg(windows)]
        let directory = {
            use std::os::windows::fs::FileTypeExt;
            kind.is_symlink_dir()
        };
        #[cfg(not(windows))]
        let directory = false;
        return Ok(Fingerprint::Link {
            target: target.into(),
            directory,
        });
    }
    if kind.is_dir() {
        if disk::linked(&metadata) {
            return Err(format!("Unsupported reparse point {name}"));
        }
        return Ok(Fingerprint::Directory {
            mode: mode(&metadata),
        });
    }
    // Other Windows reparse points, such as cloud-file placeholders, are
    // file content, as when the copy was made.
    if !kind.is_file() {
        return Err(format!("Unsupported filesystem entry {name}"));
    }
    let (size, digest) = observe_file(&path).map_err(|error| format!("{name}: {error}"))?;
    let later = fs::symlink_metadata(&path).map_err(|error| error.to_string())?;
    if !later.is_file() || later.len() != size || mode(&later) != mode(&metadata) {
        return Err(format!("{name} changed while reading"));
    }
    Ok(Fingerprint::File {
        size,
        digest,
        mode: mode(&metadata),
    })
}

/// Hash a regular file's content: its length and digest.
pub(super) fn observe_file(path: &Path) -> Result<(u64, String), String> {
    let mut file = File::open(path).map_err(|error| format!("Could not open: {error}"))?;
    let size = file.metadata().map_err(|error| error.to_string())?.len();
    let mut hasher = hash::Hasher::new(size);
    let mut remaining = size;
    let mut buffer = vec![0u8; 64 * 1024];
    while remaining > 0 {
        let wanted = remaining.min(buffer.len() as u64) as usize;
        let count = file
            .read(&mut buffer[..wanted])
            .map_err(|error| format!("Could not read: {error}"))?;
        if count == 0 {
            return Err("changed while reading".into());
        }
        hasher.update(&buffer[..count]);
        remaining -= count as u64;
    }
    Ok((size, hex(hasher.finish())))
}

pub(super) fn hex(digest: hash::Digest) -> String {
    format!("{:016x}{:016x}", digest.0[0], digest.0[1])
}

pub(super) fn mode(metadata: &fs::Metadata) -> u32 {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o7777
    }
    #[cfg(not(unix))]
    {
        u32::from(metadata.permissions().readonly())
    }
}

pub(super) fn set_mode(permissions: &mut fs::Permissions, mode: u32) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        permissions.set_mode(mode);
    }
    #[cfg(not(unix))]
    permissions.set_readonly(mode != 0);
}

pub(super) fn apply_mode(path: &Path, mode: u32) -> Result<(), String> {
    let mut permissions = fs::symlink_metadata(path)
        .map_err(|error| error.to_string())?
        .permissions();
    set_mode(&mut permissions, mode);
    fs::set_permissions(path, permissions).map_err(|error| error.to_string())
}

/// Give the owner the access needed to list and delete a directory's
/// entries, or to delete a read-only Windows file.
fn writable(path: &Path, metadata: &fs::Metadata) -> Result<(), String> {
    let mut permissions = metadata.permissions();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = permissions.mode();
        if !metadata.is_dir() || mode & 0o700 == 0o700 {
            return Ok(());
        }
        permissions.set_mode(mode | 0o700);
    }
    #[cfg(not(unix))]
    {
        if !permissions.readonly() {
            return Ok(());
        }
        set_mode(&mut permissions, 0);
    }
    fs::set_permissions(path, permissions)
        .map_err(|error| format!("Could not make {} writable: {error}", path.display()))
}

pub(super) fn remove_baseline(base: &Path) -> Result<(), String> {
    let metadata = fs::symlink_metadata(base).map_err(|error| error.to_string())?;
    if !disk::ordinary_directory(&metadata) {
        return Err("Workspace baseline is not an ordinary directory".into());
    }
    let mut entries = Vec::new();
    let mut pending = vec![(base.to_path_buf(), false)];
    while let Some((path, visited)) = pending.pop() {
        let metadata = fs::symlink_metadata(&path).map_err(|error| error.to_string())?;
        if disk::ordinary_directory(&metadata) {
            if visited {
                entries.push((path, true));
            } else {
                pending.push((path.clone(), true));
                for child in fs::read_dir(&path).map_err(|error| error.to_string())? {
                    pending.push((child.map_err(|error| error.to_string())?.path(), false));
                }
            }
        } else if metadata.is_file() && !disk::linked(&metadata) {
            entries.push((path, false));
        } else {
            return Err("Workspace baseline contains a link or special file".into());
        }
    }
    for (path, directory) in entries {
        if directory {
            fs::remove_dir(&path).map_err(|error| error.to_string())?;
        } else {
            fs::remove_file(&path).map_err(|error| error.to_string())?;
        }
    }
    Ok(())
}

/// Delete only entries inside an owned copy. Inspect the entire tree before
/// removing anything, and never follow directory links or reparse points.
/// Restrictive directory modes are lifted first so their entries can go.
pub(super) fn remove_tree(root: &Path) -> Result<(), String> {
    let metadata = fs::symlink_metadata(root).map_err(|error| error.to_string())?;
    if !disk::ordinary_directory(&metadata) {
        return Err("Workspace copy is not an ordinary directory".into());
    }
    enum Removal {
        Directory,
        DirectoryLink,
        Entry,
    }
    let mut entries = Vec::new();
    let mut pending = vec![(root.to_path_buf(), false)];
    while let Some((path, visited)) = pending.pop() {
        let metadata = fs::symlink_metadata(&path).map_err(|error| error.to_string())?;
        let kind = metadata.file_type();
        if disk::ordinary_directory(&metadata) {
            if visited {
                entries.push((path, Removal::Directory));
            } else {
                writable(&path, &metadata)?;
                pending.push((path.clone(), true));
                for child in fs::read_dir(&path).map_err(|error| error.to_string())? {
                    pending.push((child.map_err(|error| error.to_string())?.path(), false));
                }
            }
        } else if kind.is_symlink() {
            #[cfg(windows)]
            let directory = std::os::windows::fs::FileTypeExt::is_symlink_dir(&kind);
            #[cfg(not(windows))]
            let directory = false;
            entries.push((
                path,
                if directory {
                    Removal::DirectoryLink
                } else {
                    Removal::Entry
                },
            ));
        } else if !metadata.is_dir() {
            // Files, including sockets or pipes a command left in the copy.
            #[cfg(windows)]
            writable(&path, &metadata)?;
            entries.push((path, Removal::Entry));
        } else {
            return Err(format!(
                "Workspace copy has an unsupported entry: {}",
                path.display()
            ));
        }
    }
    for (path, removal) in entries {
        match removal {
            Removal::Directory | Removal::DirectoryLink => fs::remove_dir(&path),
            Removal::Entry => fs::remove_file(&path),
        }
        .map_err(|error| format!("Could not remove {}: {error}", path.display()))?;
    }
    Ok(())
}

pub(super) fn verify_content(
    root: &Path,
    name: &str,
    expected: &Fingerprint,
) -> Result<(), String> {
    if matches!(
        expected,
        Fingerprint::Link { .. } | Fingerprint::Directory { .. }
    ) {
        return Ok(());
    }
    let observed = observe(root, name)?;
    match (expected, observed) {
        (
            Fingerprint::File {
                size: old_size,
                digest: old_digest,
                ..
            },
            Fingerprint::File { size, digest, .. },
        ) if *old_size == size && *old_digest == digest => Ok(()),
        (Fingerprint::Absent, Fingerprint::Absent) => Ok(()),
        _ => Err(format!(
            "{name} content differs from the reviewed workspace state"
        )),
    }
}
