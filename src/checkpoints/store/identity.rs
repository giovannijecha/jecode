//! Workspace ownership and physical archive exclusions.
use super::super::hash;
use std::fs;
use std::path::{Component, Path};

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
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Ok(false);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            return Ok(false);
        }
    }
    Ok(true)
}

pub(super) fn ordinary_file(path: &Path) -> Result<bool, String> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(format!("Could not inspect recovery pack: {error}")),
    };
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Ok(false);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            return Ok(false);
        }
    }
    Ok(true)
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
    let marker = directory.join("workspace.id");
    let metadata = match fs::symlink_metadata(&marker) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(format!("Could not inspect recovery identity: {error}")),
    };
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() > 65_536 {
        return Ok(false);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            return Ok(false);
        }
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
