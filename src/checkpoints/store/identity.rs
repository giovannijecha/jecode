//! Workspace ownership and physical archive exclusions.
use super::super::{files, hash, locks};
use std::fs;
use std::path::{Component, Path, PathBuf};

/// Establish the only current layout. A missing ownership marker may be
/// completed after interrupted initialization, but never over unrelated data.
pub(super) fn open_workspace(home: &Path, root: &Path) -> Result<PathBuf, String> {
    crate::disk::private_directory(home)
        .map_err(|e| format!("Could not create Jecode home: {e}"))?;
    let home = fs::canonicalize(home).map_err(|e| e.to_string())?;
    let checkpoints = home.join("checkpoints");
    private_directory(&checkpoints)?;
    let identity = identity_bytes(root);
    let key = hash::digest(&identity);
    let leaf = root
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("workspace");
    let friendly: String = leaf
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
        .take(32)
        .collect();
    let friendly = if friendly.is_empty() {
        "workspace"
    } else {
        &friendly
    };
    let directory = checkpoints.join(format!("{friendly}-{:016x}{:016x}", key.0[0], key.0[1]));
    private_directory(&directory)?;
    let internal = directory.join(".internal");
    let marker = internal.join("workspace.id");
    // Refuse an occupied, unmarked archive before adding our metadata to it.
    if !internal.exists() {
        for entry in fs::read_dir(&directory).map_err(|e| e.to_string())? {
            if entry.map_err(|e| e.to_string())?.file_name() != ".internal" && !marker.exists() {
                return Err("Checkpoint directory contains unrecognized data".into());
            }
        }
    }
    private_directory(&internal)?;
    let lock = internal.join("identity.lock");
    if fs::symlink_metadata(&lock).is_ok() && !ordinary_file(&lock)? {
        return Err("Checkpoint identity lock is not an ordinary file".into());
    }
    let _held = locks::Exclusive::acquire(&lock).map_err(|e| e.to_string())?;
    match fs::symlink_metadata(&marker) {
        Ok(metadata) => {
            if !ordinary_file(&marker)?
                || metadata.len() != identity.len() as u64
                || fs::read(&marker).map_err(|e| e.to_string())? != identity
            {
                return Err(
                    "Checkpoint directory belongs to another workspace or has invalid identity"
                        .into(),
                );
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let root_entries = fs::read_dir(&directory).map_err(|e| e.to_string())?;
            for entry in root_entries {
                if entry.map_err(|e| e.to_string())?.file_name() != ".internal" {
                    return Err("Checkpoint directory contains unrecognized data".into());
                }
            }
            let mut abandoned = Vec::new();
            for entry in fs::read_dir(&internal).map_err(|e| e.to_string())? {
                let entry = entry.map_err(|e| e.to_string())?;
                if entry.file_name() == "identity.lock" {
                    continue;
                }
                if !initialization_temporary(&entry.path()) || !ordinary_file(&entry.path())? {
                    return Err("Checkpoint metadata has no valid workspace identity".into());
                }
                abandoned.push(entry.path());
            }
            // Only the identity-lock holder can initialize this archive.
            // A previous holder died before publishing its marker.
            for temporary in abandoned {
                fs::remove_file(temporary).map_err(|e| e.to_string())?;
            }
            files::atomic_write(&marker, &identity).map_err(|e| e.to_string())?;
        }
        Err(error) => return Err(format!("Could not read checkpoint identity: {error}")),
    }
    private_directory(&internal.join("leases"))?;
    fs::canonicalize(directory).map_err(|e| e.to_string())
}

fn initialization_temporary(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    let Some(parts) = name
        .strip_prefix(".write-")
        .and_then(|name| name.strip_suffix(".tmp"))
    else {
        return false;
    };
    let mut parts = parts.split('-');
    parts.next().is_some_and(|part| part.parse::<u32>().is_ok())
        && parts.next().is_some_and(|part| part.parse::<u64>().is_ok())
        && parts.next().is_some_and(|part| part.parse::<u64>().is_ok())
        && parts.next().is_none()
}

fn private_directory(path: &Path) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Ok(_) if ordinary_directory(path)? => Ok(()),
        Ok(_) => Err(format!(
            "Checkpoint storage {} is not an ordinary directory",
            path.display()
        )),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            crate::disk::private_directory(path).map_err(|e| e.to_string())?;
            if !ordinary_directory(path)? {
                return Err("Checkpoint storage changed during creation".into());
            }
            files::sync_directory(path.parent().unwrap()).map_err(|e| e.to_string())
        }
        Err(error) => Err(format!("Could not inspect checkpoint storage: {error}")),
    }
}

pub(super) fn add_exclusion(root: &Path, path: &Path, excluded: &mut Vec<String>) {
    let mut parts = path.components();
    for expected in root.components() {
        let Some(actual) = parts.next() else { return };
        #[cfg(windows)]
        if actual.as_os_str().to_string_lossy().to_lowercase()
            != expected.as_os_str().to_string_lossy().to_lowercase()
        {
            return;
        }
        #[cfg(not(windows))]
        if actual != expected {
            return;
        }
    }
    let rest: Option<Vec<_>> = parts
        .map(|part| match part {
            Component::Normal(name) => name.to_str(),
            _ => None,
        })
        .collect();
    if let Some(rest) = rest {
        excluded.push(rest.join("/"));
    }
}

pub(super) fn ordinary_directory(path: &Path) -> Result<bool, String> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(format!("Could not inspect recovery directory: {error}")),
    };
    Ok(metadata.is_dir() && !crate::disk::linked(&metadata))
}

pub(super) fn ordinary_file(path: &Path) -> Result<bool, String> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(format!("Could not inspect recovery pack: {error}")),
    };
    Ok(metadata.is_file() && !crate::disk::linked(&metadata))
}

pub(super) fn recognized_workspace(directory: &Path) -> Result<bool, String> {
    let Some(name) = directory.file_name().and_then(|name| name.to_str()) else {
        return Ok(false);
    };
    let Some((friendly, digest)) = name.rsplit_once('-') else {
        return Ok(false);
    };
    if friendly.is_empty()
        || friendly.len() > 32
        || !friendly
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        || digest.len() != 32
        || !digest.bytes().all(|b| b.is_ascii_hexdigit())
    {
        return Ok(false);
    }
    if !ordinary_directory(&directory.join(".internal"))? {
        return Ok(false);
    }
    let marker = directory.join(".internal/workspace.id");
    let metadata = match fs::symlink_metadata(&marker) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(format!("Could not inspect recovery identity: {error}")),
    };
    if !metadata.is_file() || crate::disk::linked(&metadata) || metadata.len() > 65_536 {
        return Ok(false);
    }
    let bytes = fs::read(&marker).map_err(|e| format!("Could not read recovery identity: {e}"))?;
    let key = hash::digest(&bytes);
    Ok(format!("{:016x}{:016x}", key.0[0], key.0[1]) == digest)
}

pub(super) fn identity_bytes(path: &Path) -> Vec<u8> {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        path.as_os_str()
            .encode_wide()
            .flat_map(u16::to_le_bytes)
            .collect()
    }
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        path.as_os_str().as_bytes().to_vec()
    }
}
