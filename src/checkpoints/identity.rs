//! Workspace identity: one archive directory per canonical workspace root.
use super::files;
use crate::json::{self, Value};
use std::fs;
use std::path::{Path, PathBuf};

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
    let base = key.bytes().fold(0u128, |value, byte| {
        value.wrapping_mul(257).wrapping_add(u128::from(byte) + 1)
    });
    let archives = home.join("checkpoints");
    let node = node(&root);
    let mut free = None;
    for suffix in 0..64 {
        let id = if suffix == 0 {
            format!("{base:032x}")
        } else {
            format!("{base:032x}-{suffix}")
        };
        let directory = archives.join(&id);
        match read(&directory) {
            Some(record) if record.root == key => {
                return Ok(Some(Location { root, directory }));
            }
            Some(_) => {}
            None if directory.exists() && directory.join("workspace.json").exists() => {}
            None => {
                free.get_or_insert(directory);
            }
        }
    }
    if let Some(location) = node.and_then(|node| alias(&archives, &root, node)) {
        return Ok(Some(location));
    }
    if !create {
        return Ok(None);
    }
    let directory = free.ok_or("Too many checkpoint archives share one identity hash")?;
    files::private_directory(&directory)
        .map_err(|error| format!("Could not create checkpoint archive: {error}"))?;
    let mut record = vec![
        ("version", Value::number(1)),
        ("root", Value::string(&key)),
        ("created", Value::number(crate::sessions::now())),
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
    #[cfg(windows)]
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

/// The directory entry's spelling on Windows. The final entry may be a link
/// or junction, so only ordinary files and directories can be canonicalized.
#[cfg(windows)]
fn existing_name(path: &Path) -> Option<std::ffi::OsString> {
    use std::os::windows::fs::MetadataExt;

    let metadata = fs::symlink_metadata(path).ok()?;
    const REPARSE_POINT: u32 = 0x400;
    if metadata.file_attributes() & REPARSE_POINT == 0 {
        return fs::canonicalize(path)
            .ok()?
            .file_name()
            .map(std::ffi::OsStr::to_os_string);
    }
    let requested = path.file_name()?;
    let folded = requested.to_string_lossy().to_lowercase();
    let mut folded_match = None;
    for entry in fs::read_dir(path.parent()?).ok()?.flatten() {
        let name = entry.file_name();
        if name == requested {
            return Some(name);
        }
        if name.to_string_lossy().to_lowercase() == folded {
            folded_match = Some(name);
        }
    }
    folded_match
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
mod tests {
    use super::*;
    use crate::test_support::Directory;

    #[test]
    fn equivalent_paths_share_one_archive_and_collisions_get_a_suffix() {
        let fixture = Directory::new();
        let home = fixture.path().join("home");
        let workspace = fixture.path().join("Work");
        fs::create_dir_all(workspace.join("sub")).unwrap();
        let first = locate(&home, &workspace, true).unwrap().unwrap();
        let again = locate(&home, &workspace.join("sub").join(".."), false)
            .unwrap()
            .unwrap();
        assert_eq!(first.directory, again.directory);
        #[cfg(windows)]
        {
            let alias = fixture.path().join("WORK");
            let other = locate(&home, &alias, false).unwrap().unwrap();
            assert_eq!(first.directory, other.directory);
        }
        #[cfg(unix)]
        {
            let link = fixture.path().join("link");
            std::os::unix::fs::symlink(&workspace, &link).unwrap();
            let other = locate(&home, &link, false).unwrap().unwrap();
            assert_eq!(first.directory, other.directory);
        }
        // Simulate a different root that hashes to the same archive name.
        fs::write(
            first.directory.join("workspace.json"),
            r#"{"version":1,"root":"elsewhere"}"#,
        )
        .unwrap();
        let moved = locate(&home, &workspace, true).unwrap().unwrap();
        assert_ne!(moved.directory, first.directory);
        assert!(moved.directory.to_string_lossy().ends_with("-1"));
        assert!(locate(&home, &fixture.path().join("missing"), false).is_err());
    }

    #[test]
    fn relative_paths_resolve_aliases_without_following_the_entry() {
        let fixture = Directory::new();
        fs::create_dir_all(fixture.path().join("Work").join("src")).unwrap();
        let root = fs::canonicalize(fixture.path().join("Work")).unwrap();
        let plain_root = plain(&root);
        assert_eq!(
            relative(&root, &absolute(&plain_root, "src/new/b.txt")).as_deref(),
            Some("src/new/b.txt")
        );
        assert_eq!(relative(&root, &root), None);
        assert_eq!(relative(&root, &plain_root.join("..").join("Other")), None);
        #[cfg(windows)]
        {
            assert!(!plain_root.to_string_lossy().starts_with(r"\\?\"));
            let alias = fixture.path().join("WORK").join("SRC").join("a.rs");
            assert_eq!(relative(&root, &alias).as_deref(), Some("src/a.rs"));
            fs::write(root.join("src/Case.txt"), b"case").unwrap();
            let file_alias = fixture.path().join("WORK/SRC/cASE.TXT");
            assert_eq!(
                relative(&root, &file_alias).as_deref(),
                Some("src/Case.txt")
            );
            // The last entry itself is never followed, and a new file keeps
            // the spelling supplied by its creator.
            assert_eq!(
                relative(&root, &fixture.path().join("WORK/SRC/New.txt")).as_deref(),
                Some("src/New.txt")
            );
        }
        #[cfg(unix)]
        {
            let link = fixture.path().join("link");
            std::os::unix::fs::symlink(&root, &link).unwrap();
            assert_eq!(
                relative(&root, &link.join("src/a.rs")).as_deref(),
                Some("src/a.rs")
            );
            // A link inside the workspace is the entry itself.
            std::os::unix::fs::symlink("/elsewhere", root.join("src/out")).unwrap();
            assert_eq!(
                relative(&root, &root.join("src/out")).as_deref(),
                Some("src/out")
            );
        }
    }
}
