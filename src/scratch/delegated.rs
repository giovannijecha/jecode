//! Read-only lookups in another agent's temporary area, for a main agent
//! reading what its collaborators wrote: nothing is created and no link or
//! junction is followed.

use super::{OWNERSHIP, relative};
use crate::json::Value;
use std::fs::{self, Metadata};
use std::path::{Path, PathBuf};

/// The area root and file of `alias` in the existing temporary area of
/// `session` under `home`, the `tmp` directory of that session's home.
/// Entries below `boundary` must be ordinary, and the area must record
/// `session`. A linked folder of areas is skipped without being entered.
pub fn find(
    boundary: &Path,
    home: &Path,
    session: &str,
    alias: &str,
) -> Result<Option<(PathBuf, PathBuf)>, String> {
    let relative = relative(alias)?;
    let below = home
        .strip_prefix(boundary)
        .map_err(|_| "Invalid temporary area location")?;
    let mut path = boundary.to_path_buf();
    for part in below.components() {
        path.push(part);
        if !present(&path)?.is_some_and(|metadata| metadata.is_dir()) {
            return Ok(None);
        }
    }
    for folder in fs::read_dir(home).map_err(failure)? {
        let folder = folder.map_err(failure)?.path();
        let ordinary = crate::disk::metadata_if_present(&folder)
            .ok()
            .flatten()
            .is_some_and(|metadata| crate::disk::ordinary_directory(&metadata));
        let area = folder.join(session);
        let Some(metadata) = ordinary.then(|| present(&area)).transpose()?.flatten() else {
            continue;
        };
        if !metadata.is_dir() {
            return Err("Temporary area must be a directory".into());
        }
        verify(&area, session)?;
        let root = fs::canonicalize(&area).map_err(failure)?;
        let mut path = root.clone();
        for part in relative.components() {
            path.push(part.as_os_str());
            if present(&path)?.is_none() {
                return Ok(None);
            }
        }
        return Ok(Some((root, path)));
    }
    Ok(None)
}

/// The area's record names `session` and the folder that session worked in.
fn verify(area: &Path, session: &str) -> Result<(), String> {
    let recorded = OWNERSHIP.read(area)?;
    let project = recorded
        .get("directory")
        .and_then(Value::as_str)
        .ok_or("Invalid temporary ownership record")?;
    if recorded != OWNERSHIP.identity(Path::new(project), session) {
        return Err(
            "Temporary area belongs to another folder or session; it was left unchanged".into(),
        );
    }
    Ok(())
}

fn present(path: &Path) -> Result<Option<Metadata>, String> {
    let metadata = crate::disk::metadata_if_present(path).map_err(failure)?;
    if metadata.as_ref().is_some_and(crate::disk::linked) {
        return Err("Temporary area contains a link or junction; it was not followed".into());
    }
    Ok(metadata)
}

fn failure(error: std::io::Error) -> String {
    format!("Could not inspect temporary path: {error}")
}
