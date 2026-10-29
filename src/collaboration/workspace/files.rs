use crate::disk::{self, hash};
use crate::json::Value;
use std::fs::{self, File};
use std::io::{Read, Write};
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
    Directory,
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
            Self::Directory => "directory",
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
            Self::Directory => Value::object([("kind", Value::string("directory"))]),
        }
    }
    pub(super) fn from_value(value: &Value) -> Result<Self, String> {
        match value.get("kind").and_then(Value::as_str) {
            Some("absent") => Ok(Self::Absent),
            Some("directory") => Ok(Self::Directory),
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

fn parent(root: &Path, name: &str, create: bool) -> Result<PathBuf, String> {
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
            None if create => fs::create_dir(&result)
                .map_err(|error| format!("Could not create parent of {name}: {error}"))?,
            None => return Ok(result),
        }
    }
    Ok(result)
}

pub(super) fn observe(root: &Path, name: &str) -> Result<Fingerprint, String> {
    parent(root, name, false)?;
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
    if disk::linked(&metadata) {
        return Err(format!("Unsupported reparse point {name}"));
    }
    if kind.is_dir() {
        return Ok(Fingerprint::Directory);
    }
    if !kind.is_file() {
        return Err(format!("Unsupported filesystem entry {name}"));
    }
    let mut file = File::open(&path).map_err(|error| format!("Could not open {name}: {error}"))?;
    let size = metadata.len();
    let mut hasher = hash::Hasher::new(size);
    let mut remaining = size;
    let mut buffer = [0u8; 64 * 1024];
    while remaining > 0 {
        let wanted = remaining.min(buffer.len() as u64) as usize;
        let count = file
            .read(&mut buffer[..wanted])
            .map_err(|error| format!("Could not read {name}: {error}"))?;
        if count == 0 {
            return Err(format!("{name} changed while reading"));
        }
        hasher.update(&buffer[..count]);
        remaining -= count as u64;
    }
    let later = fs::symlink_metadata(&path).map_err(|error| error.to_string())?;
    if !later.is_file() || later.len() != size || mode(&later) != mode(&metadata) {
        return Err(format!("{name} changed while reading"));
    }
    let digest = hasher.finish();
    Ok(Fingerprint::File {
        size,
        digest: format!("{:016x}{:016x}", digest.0[0], digest.0[1]),
        mode: mode(&metadata),
    })
}

fn mode(metadata: &fs::Metadata) -> u32 {
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

fn apply_mode(path: &Path, mode: u32) -> Result<(), String> {
    let mut permissions = fs::symlink_metadata(path)
        .map_err(|error| error.to_string())?
        .permissions();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        permissions.set_mode(mode);
    }
    #[cfg(not(unix))]
    permissions.set_readonly(mode != 0);
    fs::set_permissions(path, permissions).map_err(|error| error.to_string())
}

fn remove(path: &Path) -> Result<(), String> {
    let Some(metadata) = disk::metadata_if_present(path).map_err(|error| error.to_string())? else {
        return Ok(());
    };
    if disk::ordinary_directory(&metadata) {
        fs::remove_dir(path).map_err(|error| error.to_string())
    } else if metadata.file_type().is_symlink() && metadata.is_dir() {
        fs::remove_dir(path).map_err(|error| error.to_string())
    } else {
        #[cfg(windows)]
        if metadata.is_file() && metadata.permissions().readonly() {
            apply_mode(path, 0)?;
        }
        fs::remove_file(path).map_err(|error| error.to_string())
    }
}

pub(super) fn copy_entry(
    source: &Path,
    target: &Path,
    name: &str,
    expected: &Fingerprint,
) -> Result<(), String> {
    parent(source, name, false)?;
    parent(target, name, true)?;
    let from = path(source, name)?;
    let to = path(target, name)?;
    if observe(source, name)? != *expected {
        return Err(format!("{name} changed before copy"));
    }
    match expected {
        Fingerprint::Absent => remove(&to)?,
        // Submodules and nested repositories are separate repositories: the
        // copy keeps their directory, not their contents.
        Fingerprint::Directory => {
            if observe(target, name)? != Fingerprint::Directory {
                remove(&to)?;
                fs::create_dir(&to).map_err(|error| error.to_string())?;
            }
        }
        Fingerprint::File { mode, .. } => {
            remove(&to)?;
            let staged = disk::temporary_beside(&to);
            let result = (|| {
                let mut input = File::open(&from).map_err(|error| error.to_string())?;
                let mut output = fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&staged)
                    .map_err(|error| error.to_string())?;
                std::io::copy(&mut input, &mut output).map_err(|error| error.to_string())?;
                output.flush().map_err(|error| error.to_string())?;
                apply_mode(&staged, *mode)?;
                fs::rename(&staged, &to).map_err(|error| error.to_string())
            })();
            if result.is_err() {
                let _ = fs::remove_file(&staged);
            }
            result?;
        }
        Fingerprint::Link { target, directory } => {
            remove(&to)?;
            #[cfg(unix)]
            std::os::unix::fs::symlink(target, &to).map_err(|error| error.to_string())?;
            #[cfg(windows)]
            {
                if *directory {
                    std::os::windows::fs::symlink_dir(target, &to)
                } else {
                    std::os::windows::fs::symlink_file(target, &to)
                }
                .map_err(|error| error.to_string())?;
            }
            #[cfg(not(windows))]
            let _ = directory;
        }
    }
    if observe(source, name)? != *expected || observe(target, name)? != *expected {
        return Err(format!("{name} changed while copying"));
    }
    Ok(())
}

pub(super) fn save_baseline(
    source: &Path,
    base: &Path,
    name: &str,
    expected: &Fingerprint,
) -> Result<(), String> {
    if !matches!(expected, Fingerprint::File { .. }) {
        return Ok(());
    }
    let from = path(source, name)?;
    let to = path(base, name)?;
    if let Some(parent) = to.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let mut input = File::open(&from).map_err(|error| error.to_string())?;
    let mut output = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&to)
        .map_err(|error| error.to_string())?;
    std::io::copy(&mut input, &mut output).map_err(|error| error.to_string())?;
    output.sync_all().map_err(|error| error.to_string())?;
    if observe(source, name)? != *expected {
        return Err(format!("{name} changed while saving baseline"));
    }
    Ok(())
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

/// Removes the links a collaborator left in its copy without following them,
/// so deleting the copy never depends on how Git treats links on this platform.
pub(super) fn remove_links(root: &Path) -> Result<(), String> {
    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(&directory).map_err(|error| error.to_string())? {
            let entry = entry.map_err(|error| error.to_string())?;
            let kind = entry.file_type().map_err(|error| error.to_string())?;
            let path = entry.path();
            if kind.is_symlink() {
                #[cfg(windows)]
                let removed = if std::os::windows::fs::FileTypeExt::is_symlink_dir(&kind) {
                    fs::remove_dir(&path)
                } else {
                    fs::remove_file(&path)
                };
                #[cfg(not(windows))]
                let removed = fs::remove_file(&path);
                removed.map_err(|error| {
                    format!("Could not remove link {}: {error}", path.display())
                })?;
            } else if kind.is_dir() {
                pending.push(path);
            }
        }
    }
    Ok(())
}

pub(super) fn verify_content(
    root: &Path,
    name: &str,
    expected: &Fingerprint,
) -> Result<(), String> {
    if matches!(expected, Fingerprint::Link { .. } | Fingerprint::Directory) {
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
