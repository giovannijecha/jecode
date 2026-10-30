//! Copies made by the earlier Git worktree version registered themselves in
//! the project's `.git/worktrees`. Closing such a copy removes that entry, and
//! only when the entry names this copy back.
use crate::disk;
use std::fs;
use std::path::{Path, PathBuf};

/// The project's worktree registration for `tree`, if `tree` is a Git
/// worktree made from `project`. Anything unexpected means there is none.
pub(super) fn registration(project: &Path, tree: &Path) -> Option<PathBuf> {
    let gitfile = tree.join(".git");
    let metadata = fs::symlink_metadata(&gitfile).ok()?;
    if !metadata.is_file() || disk::linked(&metadata) || metadata.len() > 4096 {
        return None;
    }
    let text = fs::read_to_string(&gitfile).ok()?;
    let gitdir = Path::new(text.trim_end().strip_prefix("gitdir: ")?);
    let registration = fs::canonicalize(tree.join(gitdir)).ok()?;
    let worktrees = fs::canonicalize(project.join(".git").join("worktrees")).ok()?;
    if registration.parent()? != worktrees
        || !disk::ordinary_directory(&fs::symlink_metadata(&registration).ok()?)
    {
        return None;
    }
    let back = fs::read_to_string(registration.join("gitdir")).ok()?;
    let back = fs::canonicalize(back.trim_end()).ok()?;
    (back == fs::canonicalize(&gitfile).ok()?).then_some(registration)
}

#[cfg(test)]
mod tests {
    use super::super::*;
    use crate::test_support::Directory;

    fn register(project: &Path, tree: &Path, name: &str, gitdir: &Path) -> PathBuf {
        let registration = project.join(".git/worktrees").join(name);
        fs::create_dir_all(registration.join("logs")).unwrap();
        fs::write(registration.join("HEAD"), "0123\n").unwrap();
        fs::write(registration.join("logs/HEAD"), "log\n").unwrap();
        fs::write(
            registration.join("gitdir"),
            format!("{}\n", gitdir.to_string_lossy().replace('\\', "/")),
        )
        .unwrap();
        fs::write(
            tree.join(".git"),
            format!(
                "gitdir: {}\n",
                registration.to_string_lossy().replace('\\', "/")
            ),
        )
        .unwrap();
        registration
    }

    #[test]
    fn closing_a_legacy_worktree_copy_removes_only_its_registration() {
        let temp = Directory::new();
        let project = temp.path().join("project");
        let storage = temp.path().join("storage");
        fs::create_dir_all(project.join(".git")).unwrap();
        fs::write(project.join("tracked.txt"), "head\n").unwrap();
        let project = fs::canonicalize(project).unwrap();
        let first = Workspace::prepare(&project, &storage, "a1", Mode::Isolated).unwrap();
        let own = register(&project, first.root(), "a1", &first.root().join(".git"));
        let second = Workspace::prepare(&project, &storage, "a2", Mode::Isolated).unwrap();
        // A registration naming another checkout is not this copy's.
        let foreign = register(
            &project,
            second.root(),
            "a2",
            &temp.path().join("elsewhere/.git"),
        );
        first.close().unwrap();
        second.close().unwrap();
        assert!(!own.exists());
        assert!(foreign.join("HEAD").exists());
        assert!(!storage.join("a1").exists() && !storage.join("a2").exists());
    }
}
