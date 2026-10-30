//! One pass over the project builds the isolated copy and its baseline. Each
//! file is read once; the same bytes are written to the copy and the saved
//! baseline and fingerprinted, so the baseline is exactly what was copied even
//! when a file changes while it is read.
use super::files::{self, Fingerprint};
use super::tree::{self, Kind, Omitted};
use crate::disk::hash;
use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{ErrorKind, Read, Write};
use std::path::{Component, Path, PathBuf};

pub(super) struct Prepared {
    pub(super) baseline: BTreeMap<String, Fingerprint>,
    pub(super) omitted: Omitted,
}

enum Outcome {
    Copied(Fingerprint),
    Vanished,
    Omitted(String),
}

#[cfg(test)]
thread_local! {
    pub(super) static FAIL: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

pub(super) fn prepare(project: &Path, tree: &Path, base: &Path) -> Result<Prepared, String> {
    let mut buffer = vec![0u8; 256 * 1024];
    let mut baseline = BTreeMap::new();
    let mut directories = Vec::new();
    let mut skipped = Vec::new();
    let mut omitted = tree::walk(project, |item| {
        let copy = files::path(tree, &item.name)?;
        let outcome = match item.kind {
            Kind::Directory => {
                let saved = files::path(base, &item.name)?;
                fs::create_dir(&copy)
                    .and_then(|()| fs::create_dir(saved))
                    .map_err(|error| format!("Could not copy {}: {error}", item.name))?;
                let state = Fingerprint::Directory {
                    mode: files::mode(&item.metadata),
                };
                directories.push((copy, state.clone()));
                Outcome::Copied(state)
            }
            Kind::File => copy_file(
                &item.path,
                &copy,
                &files::path(base, &item.name)?,
                &item.metadata,
                &mut buffer,
            )
            .map_err(|error| format!("Could not copy {}: {error}", item.name))?,
            Kind::Link => copy_link(project, tree, &item.name, &item.metadata)?,
        };
        match outcome {
            Outcome::Copied(state) => {
                baseline.insert(item.name, state);
            }
            Outcome::Vanished => {}
            Outcome::Omitted(reason) => skipped.push((item.name, reason)),
        }
        Ok(())
    })?;
    for (name, reason) in skipped {
        omitted.add(name, reason);
    }
    #[cfg(test)]
    if FAIL.with(std::cell::Cell::get) {
        return Err("Injected copy failure".into());
    }
    // Restrictive directory modes would block creating their contents.
    for (path, state) in directories.iter().rev() {
        if let Fingerprint::Directory { mode } = state {
            files::apply_mode(path, *mode)?;
        }
    }
    Ok(Prepared { baseline, omitted })
}

/// Copy one file to `copy` and `saved` from a single read. A source that
/// cannot be opened is omitted; failures writing owned storage are errors.
fn copy_file(
    source: &Path,
    copy: &Path,
    saved: &Path,
    listed: &fs::Metadata,
    buffer: &mut [u8],
) -> Result<Outcome, String> {
    let mut input = match File::open(source) {
        Ok(input) => input,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Outcome::Vanished),
        Err(error) => return Ok(Outcome::Omitted(format!("Could not read: {error}"))),
    };
    let metadata = match input.metadata() {
        Ok(metadata) if metadata.is_file() && same_file(listed, &metadata) => metadata,
        Ok(_) => return Ok(Outcome::Omitted("Replaced while copying".into())),
        Err(error) => return Ok(Outcome::Omitted(format!("Could not read: {error}"))),
    };
    let create = |path: &Path| {
        fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .map_err(|error| error.to_string())
    };
    let mut output = create(copy)?;
    let mut base = match create(saved) {
        Ok(base) => base,
        Err(error) => {
            drop(output);
            let _ = fs::remove_file(copy);
            return Err(error);
        }
    };
    let size = metadata.len();
    let mut hasher = Some(hash::Hasher::new(size));
    let mut written = 0u64;
    loop {
        let count = match input.read(buffer) {
            Ok(count) => count,
            Err(error) if error.kind() == ErrorKind::Interrupted => continue,
            Err(error) => {
                drop((output, base));
                let _ = fs::remove_file(copy);
                let _ = fs::remove_file(saved);
                return Ok(Outcome::Omitted(format!("Could not read: {error}")));
            }
        };
        if count == 0 {
            break;
        }
        let bytes = &buffer[..count];
        output.write_all(bytes).map_err(|error| error.to_string())?;
        base.write_all(bytes).map_err(|error| error.to_string())?;
        written += count as u64;
        if written > size {
            hasher = None;
        } else if let Some(state) = &mut hasher {
            state.update(bytes);
        }
    }
    let mode = files::mode(&metadata);
    let modified = metadata.modified().ok();
    // A file whose length changed while it was read is fingerprinted from
    // the saved bytes instead; either way the baseline is what was copied.
    let digest = match hasher {
        Some(state) if written == size => files::hex(state.finish()),
        _ => {
            base.flush().map_err(|error| error.to_string())?;
            drop(base);
            match files::observe_file(saved)? {
                (length, digest) if length == written => digest,
                _ => return Err("Saved baseline changed while copying".into()),
            }
        }
    };
    if let Some(modified) = modified {
        output
            .set_modified(modified)
            .map_err(|error| error.to_string())?;
    }
    let mut permissions = output
        .metadata()
        .map_err(|error| error.to_string())?
        .permissions();
    files::set_mode(&mut permissions, mode);
    output
        .set_permissions(permissions)
        .map_err(|error| error.to_string())?;
    Ok(Outcome::Copied(Fingerprint::File {
        size: written,
        digest,
        mode,
    }))
}

/// Whether an opened handle is the entry the walk listed, so a path swapped
/// for a link after listing is not followed.
fn same_file(listed: &fs::Metadata, opened: &fs::Metadata) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        listed.dev() == opened.dev() && listed.ino() == opened.ino()
    }
    #[cfg(not(unix))]
    {
        let _ = (listed, opened);
        true
    }
}

/// Recreate a link in the copy. A target inside the project is rewritten
/// relative to the link so it resolves inside the copy, never back into the
/// original; other targets are kept. A link that cannot be created, such as a
/// Windows symbolic link without the required privilege, is omitted.
fn copy_link(
    project: &Path,
    tree: &Path,
    name: &str,
    metadata: &fs::Metadata,
) -> Result<Outcome, String> {
    let source = files::path(project, name)?;
    let target = match fs::read_link(&source) {
        Ok(target) => target,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Outcome::Vanished),
        Err(error) => return Ok(Outcome::Omitted(format!("Could not read link: {error}"))),
    };
    if target.to_str().is_none() {
        return Ok(Outcome::Omitted("Link target is not valid UTF-8".into()));
    }
    let target = retarget(project, name, &target);
    let copy = files::path(tree, name)?;
    #[cfg(windows)]
    let created = {
        use std::os::windows::fs::FileTypeExt;
        if metadata.file_type().is_symlink_dir() {
            std::os::windows::fs::symlink_dir(&target, &copy)
        } else {
            std::os::windows::fs::symlink_file(&target, &copy)
        }
    };
    #[cfg(not(windows))]
    let created = {
        let _ = metadata;
        std::os::unix::fs::symlink(&target, &copy)
    };
    if let Err(error) = created {
        return Ok(Outcome::Omitted(format!("Could not create link: {error}")));
    }
    Ok(Outcome::Copied(files::observe(tree, name)?))
}

/// The target a copied link should have. `name` is the link's project path.
fn retarget(project: &Path, name: &str, target: &Path) -> PathBuf {
    let parent: Vec<&str> = name.split('/').collect();
    let parent = &parent[..parent.len() - 1];
    let source_parent = project.join(parent.join("/"));
    let path = if target.is_absolute() {
        plain(target)
    } else {
        source_parent.join(target)
    };
    let resolved = match resolve_target(&path) {
        Resolution::Full(resolved) => resolved,
        Resolution::Partial(prefix, suffix) => {
            // A prefix outside the project anchors a relative target at its
            // original location. Keep the unresolved suffix verbatim: it may
            // contain another link followed by `..`.
            let exits_early = leaves_project_before_a_name(parent.len(), target);
            return if !target.is_absolute()
                && (strip_root(&plain(project), &plain(&prefix)).is_none() || exits_early)
            {
                if exits_early {
                    path
                } else {
                    plain(&prefix.join(suffix))
                }
            } else {
                target.to_path_buf()
            };
        }
        Resolution::Unknown => return target.to_path_buf(),
    };
    let Some(inside) = strip_root(&plain(project), &plain(&resolved)) else {
        return if target.is_absolute() {
            if strip_root(&plain(project), &plain(target)).is_some() {
                plain(&resolved)
            } else {
                target.to_path_buf()
            }
        } else {
            plain(&resolved)
        };
    };
    // Relative paths must retain their spelling: an intermediate symlink can
    // change what a following `..` means after it is copied.
    if !target.is_absolute() {
        return target.to_path_buf();
    }
    let common = parent
        .iter()
        .zip(&inside)
        .take_while(|(left, right)| **left == right.as_str())
        .count();
    let mut relative = PathBuf::new();
    for _ in common..parent.len() {
        relative.push("..");
    }
    for part in &inside[common..] {
        relative.push(part);
    }
    if relative.as_os_str().is_empty() {
        relative.push(".");
    }
    relative
}

/// Leading `..` components can leave the project before any component that
/// might be a link. Such a target must stay anchored at the source project.
fn leaves_project_before_a_name(mut depth: usize, target: &Path) -> bool {
    for component in target.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir if depth == 0 => return true,
            Component::ParentDir => depth -= 1,
            _ => return false,
        }
    }
    false
}

enum Resolution {
    Full(PathBuf),
    Partial(PathBuf, PathBuf),
    Unknown,
}

/// Resolve existing components using the filesystem. A missing suffix can be
/// normalized only if no link or other filesystem error obstructed the walk.
fn resolve_target(path: &Path) -> Resolution {
    let mut uncertain = false;
    for prefix in path.ancestors() {
        match fs::canonicalize(prefix) {
            Ok(resolved) => {
                let suffix = path.strip_prefix(prefix).expect("ancestor is a prefix");
                return if uncertain {
                    Resolution::Partial(resolved, suffix.to_path_buf())
                } else {
                    Resolution::Full(lexical(&resolved.join(suffix)))
                };
            }
            Err(error) if error.kind() == ErrorKind::NotFound => {
                if fs::symlink_metadata(prefix)
                    .is_ok_and(|metadata| metadata.file_type().is_symlink())
                {
                    uncertain = true;
                }
            }
            Err(_) => uncertain = true,
        }
    }
    Resolution::Unknown
}

/// `path` relative to `root` after lexical normalization, when inside it.
fn strip_root(root: &Path, path: &Path) -> Option<Vec<String>> {
    let key = |path: &Path| {
        let text = path.to_string_lossy().into_owned();
        #[cfg(windows)]
        let text = text.to_lowercase();
        text
    };
    let path = lexical(path);
    let mut root_parts = root.components();
    let mut parts = path.components();
    loop {
        match (root_parts.next(), parts.next()) {
            (None, Some(part)) => {
                let mut inside = vec![part.as_os_str().to_string_lossy().into_owned()];
                inside.extend(parts.map(|part| part.as_os_str().to_string_lossy().into_owned()));
                return Some(inside);
            }
            (None, None) => return Some(Vec::new()),
            (Some(left), Some(right))
                if key(Path::new(left.as_os_str())) == key(Path::new(right.as_os_str())) => {}
            _ => return None,
        }
    }
}

fn lexical(path: &Path) -> PathBuf {
    let mut result = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if !matches!(
                    result.components().next_back(),
                    None | Some(Component::RootDir | Component::Prefix(_))
                ) {
                    result.pop();
                }
            }
            other => result.push(other.as_os_str()),
        }
    }
    result
}

/// Windows verbatim paths (`\\?\C:\x`, `\??\C:\x`) compared as `C:\x`.
fn plain(path: &Path) -> PathBuf {
    #[cfg(windows)]
    {
        let text = path.to_string_lossy();
        for prefix in [r"\\?\", r"\??\"] {
            if let Some(rest) = text.strip_prefix(prefix) {
                return match rest.strip_prefix("UNC\\") {
                    Some(share) => PathBuf::from(format!(r"\\{share}")),
                    None => PathBuf::from(rest),
                };
            }
        }
    }
    path.to_path_buf()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::Directory;

    #[test]
    fn links_into_the_project_are_rewritten_relative_to_the_copy() {
        let temp = Directory::new();
        let project = temp.path().join("project");
        fs::create_dir(&project).unwrap();
        fs::create_dir(temp.path().join("elsewhere")).unwrap();
        fs::create_dir(temp.path().join("sibling")).unwrap();
        let project = plain(&fs::canonicalize(project).unwrap());
        let inside = project.join("node_modules").join(".pnpm").join("pkg");
        assert_eq!(
            retarget(&project, "node_modules/pkg", &inside),
            Path::new(".pnpm").join("pkg")
        );
        assert_eq!(
            retarget(&project, "app/node_modules/pkg", &inside),
            Path::new("..")
                .join("..")
                .join("node_modules")
                .join(".pnpm")
                .join("pkg")
        );
        assert_eq!(
            retarget(&project, "a/b/link", Path::new("../c")),
            Path::new("..").join("c")
        );
        assert_eq!(
            retarget(&project, "link", Path::new("sub/./x/../y")),
            Path::new("sub/./x/../y")
        );
        assert_eq!(retarget(&project, "link", &project), Path::new("."));
        let outside = plain(&fs::canonicalize(temp.path().join("elsewhere")).unwrap());
        assert_eq!(retarget(&project, "a/link", &outside), outside);
        assert_eq!(
            retarget(&project, "link", Path::new("../sibling")),
            plain(&fs::canonicalize(temp.path().join("sibling")).unwrap())
        );
    }

    #[cfg(windows)]
    #[test]
    fn verbatim_and_case_variants_of_the_project_are_inside() {
        let temp = Directory::new();
        let project = temp.path().join("project");
        fs::create_dir_all(project.join("store/pkg")).unwrap();
        let project = plain(&fs::canonicalize(project).unwrap());
        let lower = project.to_string_lossy().to_lowercase();
        let verbatim = PathBuf::from(format!(r"\\?\{lower}\store\pkg"));
        assert_eq!(
            retarget(&project, "node_modules/pkg", &verbatim),
            Path::new("..").join("store").join("pkg")
        );
    }
}
