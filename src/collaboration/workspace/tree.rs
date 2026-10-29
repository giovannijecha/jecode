use super::files;
use crate::disk;
use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

/// Snapshot every ordinary project directory, file and symbolic link. Only
/// Jecode/Git metadata is excluded at every depth; generated root trees are
/// omitted without hiding legitimate nested paths or same-named files.
pub(super) fn inventory(root: &Path) -> Result<BTreeSet<String>, String> {
    let metadata = fs::symlink_metadata(root).map_err(|error| error.to_string())?;
    if !disk::ordinary_directory(&metadata) {
        return Err("Workspace inventory root is not an ordinary directory".into());
    }
    let mut names = BTreeSet::new();
    let mut pending = vec![(root.to_path_buf(), String::new())];
    while let Some((directory, prefix)) = pending.pop() {
        for entry in fs::read_dir(&directory).map_err(|error| error.to_string())? {
            let entry = entry.map_err(|error| error.to_string())?;
            let component = entry.file_name();
            let component = component.to_str().ok_or("Workspace has a non-UTF-8 path")?;
            if matches!(component, ".git" | ".jecode") {
                continue;
            }
            let metadata = fs::symlink_metadata(entry.path()).map_err(|error| error.to_string())?;
            if prefix.is_empty()
                && matches!(component, "target" | ".tmp")
                && disk::ordinary_directory(&metadata)
            {
                continue;
            }
            let name = if prefix.is_empty() {
                component.to_owned()
            } else {
                format!("{prefix}/{component}")
            };
            files::validate_name(&name)?;
            if disk::ordinary_directory(&metadata) {
                pending.push((entry.path(), name.clone()));
                names.insert(name);
            } else if metadata.is_file() || metadata.file_type().is_symlink() {
                names.insert(name);
            } else {
                return Err(format!("Unsupported filesystem entry {name}"));
            }
        }
    }
    Ok(names)
}
