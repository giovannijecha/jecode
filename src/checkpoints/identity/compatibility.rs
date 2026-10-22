//! Reserve the legacy lookup path and reject flat journals in a v2 archive.
use super::*;

pub(super) struct Reservation {
    pub root: String,
    pub archive: String,
}

/// A regular file occupies the directory an older Windows locator would
/// create. Older Unix locators can still find the archive through its inode,
/// so the archive also needs the root CURRENT sentinel below.
pub(super) fn reservation(path: &Path) -> Result<Option<Reservation>, String> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => return Ok(None),
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("Could not inspect checkpoint identity: {error}")),
    };
    let read = || -> Option<Reservation> {
        if !metadata.is_file() || metadata.file_type().is_symlink() {
            return None;
        }
        let value = json::parse(&fs::read_to_string(path).ok()?).ok()?;
        if value.get("kind")?.as_str()? != "checkpoint-identity"
            || value.get("version")?.as_usize()? != 1
        {
            return None;
        }
        let root = value.get("root")?.as_str()?;
        let archive = value.get("archive")?.as_str()?;
        if !Path::new(root).is_absolute() || !files::normal_component(archive) {
            return None;
        }
        Some(Reservation {
            root: root.into(),
            archive: archive.into(),
        })
    };
    read().map(Some).ok_or_else(|| {
        format!(
            "Checkpoint identity reservation at {} is invalid; history was preserved",
            path.display()
        )
    })
}

pub(super) fn reserve(path: &Path, root: &str, archive: &Path) -> Result<(), String> {
    let name = archive
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or("Checkpoint archive name is not valid text")?;
    if let Some(previous) = reservation(path)? {
        if previous.root != root {
            return Err("Checkpoint identity reservation belongs to another workspace".into());
        }
        if previous.archive == name {
            return Ok(());
        }
    } else if path.exists() {
        return Err("A legacy checkpoint archive appeared; retry opening the workspace".into());
    }
    let value = Value::object([
        ("kind", Value::string("checkpoint-identity")),
        ("version", Value::number(1)),
        ("root", Value::string(root)),
        ("archive", Value::string(name)),
    ]);
    files::atomic_write(path, value.pretty().as_bytes())
        .map_err(|error| format!("Could not reserve legacy checkpoint identity: {error}"))
}
