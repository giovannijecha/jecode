use std::fs;
use std::io::ErrorKind;
use std::path::{Component, Path, PathBuf};

/// The target a copied link should have. `name` is the link's project path.
pub(super) fn retarget(project: &Path, name: &str, target: &Path) -> PathBuf {
    let parent: Vec<&str> = name.split('/').collect();
    let parent = &parent[..parent.len() - 1];
    let source_parent = project.join(parent.join("/"));
    let path = if target.is_absolute() {
        plain(target)
    } else {
        source_parent.join(target)
    };
    let unresolved_absolute = || {
        let suffix = raw_suffix(&plain(project), &plain(target))?;
        if leaves_project_before_a_name(0, &suffix) {
            return None;
        }
        let mut relative = PathBuf::new();
        for _ in parent {
            relative.push("..");
        }
        relative.push(suffix);
        if relative.as_os_str().is_empty() {
            relative.push(".");
        }
        Some(relative)
    };
    let resolved = match resolve_target(&path) {
        Resolution::Full(resolved) => resolved,
        Resolution::Partial(prefix, suffix) => {
            if target.is_absolute() {
                return unresolved_absolute().unwrap_or_else(|| target.to_path_buf());
            }
            // A prefix outside the project anchors a relative target at its
            // original location. Keep the unresolved suffix verbatim: it may
            // contain another link followed by `..`.
            let exits_early = leaves_project_before_a_name(parent.len(), target);
            return if strip_root(&plain(project), &plain(&prefix)).is_none() || exits_early {
                if exits_early {
                    path
                } else {
                    plain(&prefix.join(suffix))
                }
            } else {
                target.to_path_buf()
            };
        }
        Resolution::Unknown => {
            return if target.is_absolute() {
                unresolved_absolute().unwrap_or_else(|| target.to_path_buf())
            } else {
                target.to_path_buf()
            };
        }
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
    let suffix = raw_suffix(root, &lexical(path))?;
    Some(
        suffix
            .components()
            .map(|part| part.as_os_str().to_string_lossy().into_owned())
            .collect(),
    )
}

/// Remove an absolute project prefix without collapsing a symlink-sensitive
/// `..` in the remaining path.
fn raw_suffix(root: &Path, path: &Path) -> Option<PathBuf> {
    let key = |path: &Path| {
        let text = path.to_string_lossy().into_owned();
        #[cfg(windows)]
        let text = text.to_lowercase();
        text
    };
    let mut root_parts = root.components();
    let mut parts = path.components();
    loop {
        match (root_parts.next(), parts.next()) {
            (None, Some(part)) => {
                let mut inside = PathBuf::from(part.as_os_str());
                inside.extend(parts.map(|part| part.as_os_str()));
                return Some(inside);
            }
            (None, None) => return Some(PathBuf::new()),
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
        assert_eq!(
            raw_suffix(&project, &project.join("alias/../goal")),
            Some(PathBuf::from("alias/../goal"))
        );
        assert!(leaves_project_before_a_name(
            0,
            &raw_suffix(&project, &project.join("../outside")).unwrap()
        ));
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
