//! Workspace identity: one archive directory per canonical workspace root.
use super::files;
use super::locks::Exclusive;
use crate::json::{self, Value};
use std::fs;
use std::path::{Path, PathBuf};

mod compatibility;

#[derive(Clone, Debug)]
pub struct Location {
    /// Canonical workspace root; the authoritative identity.
    pub root: PathBuf,
    /// Archive directory under `<home>/checkpoints/`.
    pub directory: PathBuf,
}

/// Resolves the archive for a workspace. `fs::canonicalize` resolves symbolic
/// links, junctions and Windows case aliases to one root; Unix aliases that it
/// cannot normalize, such as case-insensitive volumes, are matched through the
/// recorded device and inode. A hash collision takes the next free suffix.
pub fn locate(home: &Path, workspace: &Path, create: bool) -> Result<Option<Location>, String> {
    if !home.is_absolute() {
        return Err("The Jecode home directory must be an absolute path".into());
    }
    let root = fs::canonicalize(workspace).map_err(|error| {
        format!(
            "Could not resolve workspace {}: {error}",
            workspace.display()
        )
    })?;
    if !root.is_dir() {
        return Err(format!("Workspace {} is not a directory", root.display()));
    }
    let key = root.to_string_lossy().into_owned();
    let base = identity_hash(&key);
    let archives = home.join("checkpoints");
    if create {
        files::private_directory(&archives)
            .map_err(|error| format!("Could not create checkpoint archives: {error}"))?;
    }
    // Serializes archive creation across new processes. Existing version 1
    // archives are looked up under their original hash before allocating a
    // readable name for version 2.
    let _identity = if create {
        Some(
            Exclusive::acquire(&archives.join(".identity.lock"))
                .map_err(|error| format!("Could not lock checkpoint identities: {error}"))?,
        )
    } else {
        None
    };
    let node = node(&root);
    let name = root
        .file_name()
        .and_then(|name| name.to_str())
        .map(safe_name)
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "workspace".into());
    let mut free = None;
    let mut found = None;
    let mut legacy = None;
    let mut reserved = None;
    for suffix in 0..64 {
        for (kind, prefix) in [format!("{base:032x}"), format!("{name}-{base:032x}")]
            .into_iter()
            .enumerate()
        {
            let id = if suffix == 0 {
                prefix
            } else {
                format!("{prefix}-{suffix}")
            };
            let directory = archives.join(&id);
            if kind == 0
                && let Some(marker) = compatibility::reservation(&directory)?
            {
                if marker.root == key {
                    reserved.get_or_insert((directory, marker.archive));
                }
                continue;
            }
            match read(&directory) {
                Some(record) if record.root == key => {
                    if found
                        .as_ref()
                        .is_some_and(|(previous, _)| previous != &directory)
                    {
                        return Err(format!(
                            "Multiple checkpoint archives exist for {}; history was preserved",
                            root.display()
                        ));
                    }
                    found = Some((directory, kind == 0));
                }
                Some(_) => {}
                None if kind == 1
                    && reserved.as_ref().is_some_and(|(_, target)| target == &id)
                    && !directory.join("workspace.json").exists() =>
                {
                    free.get_or_insert(directory);
                }
                None if directory.exists() => {
                    return Err(format!(
                        "Checkpoint identity at {} is damaged or incomplete; history was preserved",
                        directory.display()
                    ));
                }
                None if kind == 1 => {
                    free.get_or_insert(directory);
                }
                None => {
                    legacy.get_or_insert(directory);
                }
            }
        }
    }
    if let Some((directory, legacy_identity)) = found {
        if create {
            if !legacy_identity {
                let slot = reserved
                    .map(|(path, _)| path)
                    .or(legacy)
                    .ok_or("No legacy checkpoint identity slot is available")?;
                compatibility::reserve(&slot, &key, &directory)?;
            }
            if super::layout::version(&directory).is_some_and(|version| version >= 2) {
                super::layout::guard_legacy(&directory)?;
            }
        }
        return Ok(Some(Location { root, directory }));
    }
    if let Some(location) = node.and_then(|node| alias(&archives, &root, node)) {
        if create && super::layout::version(&location.directory).is_some_and(|version| version >= 2)
        {
            super::layout::guard_legacy(&location.directory)?;
        }
        return Ok(Some(location));
    }
    if !create {
        return Ok(None);
    }
    let directory = free.ok_or("Too many checkpoint archives share one identity hash")?;
    let slot = reserved
        .map(|(path, _)| path)
        .or(legacy)
        .ok_or("No legacy checkpoint identity slot is available")?;
    // Older binaries do not use .identity.lock. Reserve their path before
    // publishing an identity their Unix inode fallback can discover.
    compatibility::reserve(&slot, &key, &directory)?;
    files::private_directory(&directory)
        .map_err(|error| format!("Could not create checkpoint archive: {error}"))?;
    super::layout::guard_legacy(&directory)?;
    let mut record = vec![
        ("version", Value::number(super::layout::VERSION)),
        ("root", Value::string(&key)),
        ("created", Value::number(crate::checkpoints::clock::now())),
    ];
    if let Some((device, inode)) = node {
        record.push(("device", Value::number(device)));
        record.push(("inode", Value::number(inode)));
    }
    let record = Value::Object(
        record
            .into_iter()
            .map(|(key, value)| (key.to_string(), value))
            .collect(),
    );
    files::atomic_write(
        &directory.join("workspace.json"),
        record.pretty().as_bytes(),
    )
    .map_err(|error| format!("Could not record checkpoint workspace: {error}"))?;
    files::sync_directory(&archives)
        .map_err(|error| format!("Could not flush checkpoint workspace identity: {error}"))?;
    Ok(Some(Location { root, directory }))
}

fn identity_hash(root: &str) -> u128 {
    root.bytes().fold(0u128, |value, byte| {
        value.wrapping_mul(257).wrapping_add(u128::from(byte) + 1)
    })
}

fn safe_name(name: &str) -> String {
    name.chars()
        .take(40)
        .map(|character| match character {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '-' | '_' => character,
            _ => '-',
        })
        .collect::<String>()
        .trim_matches('-')
        .to_string()
}

/// Rejects unrelated directories found during a global sweep. The workspace
/// itself need not still exist: inactivity can expire an orphaned archive.
pub fn recognized_archive(directory: &Path) -> bool {
    let Some(record) = read(directory) else {
        return false;
    };
    if !Path::new(&record.root).is_absolute() {
        return false;
    }
    let hash = identity_hash(&record.root);
    let hash = format!("{hash:032x}");
    let Some(name) = directory.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    name == hash
        || name
            .strip_prefix(&format!("{hash}-"))
            .is_some_and(|suffix| suffix.parse::<u32>().is_ok())
        || name.ends_with(&format!("-{hash}"))
        || name
            .rsplit_once(&format!("-{hash}-"))
            .is_some_and(|(_, suffix)| suffix.parse::<u32>().is_ok())
}

struct Record {
    root: String,
    node: Option<(u64, u64)>,
}

fn read(directory: &Path) -> Option<Record> {
    let text = fs::read_to_string(directory.join("workspace.json")).ok()?;
    let value = json::parse(&text).ok()?;
    let number = |key| {
        value
            .get(key)
            .and_then(Value::as_str)
            .map(str::to_string)
            .or_else(|| match value.get(key) {
                Some(Value::Number(number)) => Some(number.clone()),
                _ => None,
            })
            .and_then(|text| text.parse::<u64>().ok())
    };
    let version = number("version")?;
    if !(1..=super::layout::VERSION).contains(&version) {
        return None;
    }
    Some(Record {
        root: value.get("root")?.as_str()?.to_string(),
        node: number("device").zip(number("inode")),
    })
}

fn alias(archives: &Path, root: &Path, node: (u64, u64)) -> Option<Location> {
    for entry in fs::read_dir(archives).ok()?.flatten() {
        let directory = entry.path();
        let Some(record) = read(&directory) else {
            continue;
        };
        // An inode can be reused after deletion; require the recorded root to
        // still name this same directory.
        if record.node == Some(node) && self::node(Path::new(&record.root)) == Some(node) {
            return Some(Location {
                root: root.to_path_buf(),
                directory,
            });
        }
    }
    None
}

fn node(_path: &Path) -> Option<(u64, u64)> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        fs::metadata(_path)
            .ok()
            .map(|metadata| (metadata.dev(), metadata.ino()))
    }
    #[cfg(not(unix))]
    None
}

/// The workspace-relative form of a path inside the canonical `root`, with
/// `/` separators. Ancestors are resolved like the root, so link, junction,
/// case and verbatim-prefix aliases name one entry; the final component is
/// the entry itself and is never followed.
pub fn relative(root: &Path, path: &Path) -> Option<String> {
    let mut missing = vec![path.file_name()?.to_os_string()];
    if let Some(spelling) = existing_name(path) {
        missing[0] = spelling;
    }
    let mut existing = path.parent()?;
    let resolved = loop {
        if let Ok(resolved) = fs::canonicalize(existing) {
            break resolved;
        }
        missing.push(existing.file_name()?.to_os_string());
        existing = existing.parent()?;
    };
    let path = missing
        .into_iter()
        .rev()
        .fold(resolved, |path, name| path.join(name));
    let rest = path.strip_prefix(root).ok()?;
    let mut parts = Vec::new();
    for part in rest.components() {
        match part {
            std::path::Component::Normal(name) => parts.push(name.to_str()?.to_string()),
            _ => return None,
        }
    }
    (!parts.is_empty()).then(|| parts.join("/"))
}

/// The final directory entry's spelling on case-insensitive volumes. Never
/// follow a final link or junction merely to recover its name.
fn existing_name(path: &Path) -> Option<std::ffi::OsString> {
    let metadata = fs::symlink_metadata(path).ok()?;
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const REPARSE_POINT: u32 = 0x400;
        if metadata.file_attributes() & REPARSE_POINT != 0 {
            let requested = path.file_name()?;
            let folded = requested.to_string_lossy().to_lowercase();
            let mut match_name = None;
            for entry in fs::read_dir(path.parent()?).ok()?.flatten() {
                let name = entry.file_name();
                if name == requested {
                    return Some(name);
                }
                if name.to_string_lossy().to_lowercase() == folded {
                    if match_name.is_some() {
                        return None;
                    }
                    match_name = Some(name);
                }
            }
            return match_name;
        }
    }
    if metadata.file_type().is_symlink() {
        return None;
    }
    fs::canonicalize(path)
        .ok()?
        .file_name()
        .map(std::ffi::OsStr::to_os_string)
}

pub fn absolute(root: &Path, relative: &str) -> PathBuf {
    relative
        .split('/')
        .fold(root.to_path_buf(), |path, part| path.join(part))
}

/// A canonical Windows path without the verbatim prefix, for programs and
/// people that do not accept it. Other paths are returned unchanged.
pub fn plain(path: &Path) -> PathBuf {
    let text = path.to_string_lossy();
    if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
        return PathBuf::from(format!(r"\\{rest}"));
    }
    match text.strip_prefix(r"\\?\") {
        Some(rest) if rest.as_bytes().get(1) == Some(&b':') => PathBuf::from(rest),
        _ => path.to_path_buf(),
    }
}

#[cfg(test)]
mod compatibility_tests;
#[cfg(test)]
mod tests;
