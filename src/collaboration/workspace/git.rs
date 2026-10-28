//! Native Git is used only to create an independent local checkout and list its files.
use std::collections::BTreeSet;
use std::path::Path;
use std::process::{Command, Output};

fn run(project: &Path, arguments: &[&str]) -> Result<Output, String> {
    let mut command = Command::new("git");
    #[cfg(windows)]
    command.arg("-c").arg("core.longpaths=true");
    let output = command
        .arg("-C")
        .arg(project)
        .args(arguments)
        .output()
        .map_err(|error| format!("Could not run Git for isolated workspace: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "Git {} failed: {}",
            arguments.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(output)
}

pub(super) fn require_repository(project: &Path) -> Result<(), String> {
    let output = run(project, &["rev-parse", "--show-toplevel"])?;
    let root = String::from_utf8(output.stdout).map_err(|_| "Git root has an unsupported name")?;
    let root = std::fs::canonicalize(root.trim())
        .map_err(|error| format!("Could not resolve Git root: {error}"))?;
    if root != project {
        return Err("Isolated workspace requires the project Git root".into());
    }
    run(project, &["rev-parse", "--verify", "HEAD"])?;
    Ok(())
}

pub(super) fn create_worktree(project: &Path, root: &Path) -> Result<(), String> {
    #[cfg(windows)]
    let root_fs = root;
    let root = git_path(root)?;
    #[cfg(not(windows))]
    {
        run(
            project,
            &["worktree", "add", "--detach", "--", &root, "HEAD"],
        )?;
        Ok(())
    }
    #[cfg(windows)]
    {
        run(
            project,
            &[
                "worktree",
                "add",
                "--detach",
                "--no-checkout",
                "--",
                &root,
                "HEAD",
            ],
        )?;
        // Git's checkout child receives an absolute GIT_DIR derived from the
        // long worktree path and can reject it before reading config.
        // Address the linked Git directory directly for the checkout.
        let gitdir = linked_gitdir(root_fs)?;
        let gitdir_arg = git_path(&gitdir)?;
        run(
            project,
            &[
                "--git-dir",
                &gitdir_arg,
                "--work-tree",
                &root,
                "reset",
                "--hard",
                "--no-recurse-submodules",
            ],
        )
        .map_err(|error| {
            if error.contains("Filename too long") {
                format!(
                    "Git for Windows cannot prepare isolated workspace {root}: its path is too long for Git even with core.longpaths=true ({error})"
                )
            } else {
                error
            }
        })?;
        Ok(())
    }
}

#[cfg(windows)]
fn linked_gitdir(root: &Path) -> Result<std::path::PathBuf, String> {
    let gitfile = std::fs::read_to_string(root.join(".git"))
        .map_err(|error| format!("Could not read linked worktree Git file: {error}"))?;
    let gitdir = gitfile
        .trim_end()
        .strip_prefix("gitdir: ")
        .ok_or("Invalid linked worktree Git file")?;
    let gitdir = Path::new(gitdir);
    let gitdir = if gitdir.is_absolute() {
        gitdir.to_path_buf()
    } else {
        root.join(gitdir)
    };
    std::fs::canonicalize(gitdir)
        .map_err(|error| format!("Could not resolve linked worktree Git directory: {error}"))
}

pub(super) fn remove_worktree(project: &Path, root: &Path) -> Result<(), String> {
    let root = git_path(root)?;
    run(project, &["worktree", "remove", "--force", "--", &root])?;
    Ok(())
}

pub(super) fn registered(project: &Path, root: &Path) -> Result<bool, String> {
    let output = run(project, &["worktree", "list", "--porcelain", "-z"])?;
    let expected = normalized(&git_path(root)?);
    for record in output.stdout.split(|byte| *byte == 0) {
        if let Some(path) = record.strip_prefix(b"worktree ") {
            let path = std::str::from_utf8(path)
                .map_err(|_| "Git worktree list has an unsupported path")?;
            if normalized(path) == expected {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

fn normalized(path: &str) -> String {
    #[cfg(windows)]
    {
        path.replace('\\', "/").to_ascii_lowercase()
    }
    #[cfg(not(windows))]
    {
        path.into()
    }
}

fn git_path(path: &Path) -> Result<String, String> {
    let path = path
        .to_str()
        .ok_or("Workspace path has an unsupported name")?;
    #[cfg(windows)]
    {
        if let Some(path) = path.strip_prefix(r"\\?\UNC\") {
            return Ok(format!(r"\\{path}"));
        }
        if let Some(path) = path.strip_prefix(r"\\?\") {
            return Ok(path.into());
        }
    }
    Ok(path.into())
}

pub(super) fn inventory(root: &Path) -> Result<BTreeSet<String>, String> {
    let arguments = [
        "ls-files",
        "-z",
        "--cached",
        "--others",
        "--exclude-standard",
    ];
    #[cfg(windows)]
    let output = if root.join(".git").is_file() {
        let gitdir = linked_gitdir(root)?;
        let gitdir_arg = git_path(&gitdir)?;
        let root_arg = git_path(root)?;
        let mut linked = vec![
            "--git-dir",
            gitdir_arg.as_str(),
            "--work-tree",
            root_arg.as_str(),
        ];
        linked.extend(arguments);
        run(&gitdir, &linked).map_err(|error| {
            if error.contains("Filename too long") {
                format!(
                    "Git for Windows cannot inventory isolated workspace {}: its path is too long for Git even with core.longpaths=true ({error})",
                    root.display()
                )
            } else {
                error
            }
        })?
    } else {
        run(root, &arguments)?
    };
    #[cfg(not(windows))]
    let output = run(root, &arguments)?;
    let mut names = BTreeSet::new();
    for bytes in output
        .stdout
        .split(|byte| *byte == 0)
        .filter(|bytes| !bytes.is_empty())
    {
        let name = String::from_utf8(bytes.to_vec())
            .map_err(|_| "Git inventory contains a non-UTF-8 path")?;
        names.insert(name);
    }
    Ok(names)
}

pub(super) fn with_instructions(
    root: &Path,
    mut names: BTreeSet<String>,
) -> Result<BTreeSet<String>, String> {
    let mut candidates = BTreeSet::from(["AGENTS.md".to_owned()]);
    for name in &names {
        let mut parent = String::new();
        for part in name
            .split('/')
            .take(name.split('/').count().saturating_sub(1))
        {
            if !parent.is_empty() {
                parent.push('/');
            }
            parent.push_str(part);
            candidates.insert(format!("{parent}/AGENTS.md"));
        }
    }
    for candidate in candidates {
        let path = super::files::path(root, &candidate)?;
        if crate::disk::metadata_if_present(&path)
            .map_err(|error| format!("Could not inspect {candidate}: {error}"))?
            .is_some()
        {
            names.insert(candidate);
        }
    }
    Ok(names)
}
