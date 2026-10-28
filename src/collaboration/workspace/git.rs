//! Native Git is used only to create an independent local checkout and list its files.
use std::collections::BTreeSet;
use std::path::Path;
use std::process::{Command, Output};

fn run(project: &Path, arguments: &[&str]) -> Result<Output, String> {
    let output = Command::new("git")
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
    let root = git_path(root)?;
    run(
        project,
        &["worktree", "add", "--detach", "--", &root, "HEAD"],
    )?;
    Ok(())
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
    let output = run(
        root,
        &[
            "ls-files",
            "-z",
            "--cached",
            "--others",
            "--exclude-standard",
        ],
    )?;
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
