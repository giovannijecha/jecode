use super::*;
use crate::test_support::Directory;
use std::process::Command;

fn git(root: &Path, args: &[&str]) {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn fixture() -> (Directory, PathBuf, PathBuf) {
    let temp = Directory::new();
    let project = temp.path().join("project");
    let storage = temp.path().join("storage");
    fs::create_dir(&project).unwrap();
    git(&project, &["init", "-q"]);
    fs::write(project.join("tracked.txt"), "head\n").unwrap();
    git(&project, &["add", "tracked.txt"]);
    git(
        &project,
        &[
            "-c",
            "user.name=Giovanni Jecha",
            "-c",
            "user.email=100841799+giovannijecha@users.noreply.github.com",
            "commit",
            "-qm",
            "initial",
        ],
    );
    (temp, project, storage)
}

fn changed(review: &Value) -> Vec<String> {
    review
        .get("changed")
        .unwrap()
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| entry.get("path").unwrap().as_str().unwrap().to_owned())
        .collect()
}

#[cfg(windows)]
#[test]
fn isolated_windows_path_reports_git_limit_and_cleans_up() {
    for length in [240, 300] {
        let (_temp, project, mut storage) = fixture();
        while storage.join("a1/tree").to_string_lossy().len() < length {
            let remaining = length - storage.join("a1/tree").to_string_lossy().len();
            storage.push("s".repeat(remaining.saturating_sub(1).clamp(1, 24)));
        }
        assert!(storage.join("a1/tree").to_string_lossy().len() >= length);
        let workspace = match Workspace::prepare(&project, &storage, "a1", Mode::Isolated) {
            Ok(workspace) => workspace,
            Err(error) if length == 300 => {
                assert!(
                    error.contains("cannot prepare isolated workspace")
                        || error.contains("cannot inventory isolated workspace"),
                    "{error}"
                );
                assert!(error.contains("core.longpaths=true"), "{error}");
                assert_eq!(
                    fs::read_to_string(project.join("tracked.txt")).unwrap(),
                    "head\n"
                );
                assert!(!storage.join("a1").exists());
                assert!(!super::git::registered(&project, &storage.join("a1/tree")).unwrap());
                continue;
            }
            Err(error) => panic!("240-character workspace should work: {error}"),
        };
        assert_eq!(
            fs::read_to_string(workspace.root().join("tracked.txt")).unwrap(),
            "head\n"
        );
        fs::write(
            workspace.root().join("tracked.txt"),
            "changed in isolated copy\n",
        )
        .unwrap();
        let review = workspace.inspect().unwrap();
        assert_eq!(changed(&review), vec!["tracked.txt"]);
        let page = workspace.inspect_file("tracked.txt", 0, 64).unwrap();
        assert_eq!(
            page.get("before").unwrap().get("content").unwrap().as_str(),
            Some("head\n")
        );
        assert_eq!(
            page.get("after").unwrap().get("content").unwrap().as_str(),
            Some("changed in isolated copy\n")
        );
        assert_eq!(
            fs::read_to_string(project.join("tracked.txt")).unwrap(),
            "head\n"
        );
        workspace.close().unwrap();
        assert!(!workspace.root().exists());
        assert!(!storage.join("a1").exists());
        assert!(!super::git::registered(&project, &storage.join("a1/tree")).unwrap());
    }
}

#[test]
fn dirty_baseline_and_untracked_files_are_copied() {
    let (_temp, project, storage) = fixture();
    fs::write(project.join("tracked.txt"), "staged\n").unwrap();
    git(&project, &["add", "tracked.txt"]);
    fs::write(project.join("tracked.txt"), "working\n").unwrap();
    fs::write(project.join("untracked.txt"), "new\n").unwrap();
    let workspace = Workspace::prepare(&project, &storage, "a1", Mode::Isolated).unwrap();
    assert_eq!(
        fs::read_to_string(workspace.root().join("tracked.txt")).unwrap(),
        "working\n"
    );
    assert_eq!(
        fs::read_to_string(workspace.root().join("untracked.txt")).unwrap(),
        "new\n"
    );
    assert!(changed(&workspace.inspect().unwrap()).is_empty());
    let restored = Workspace::from_value(&workspace.value()).unwrap();
    assert!(changed(&restored.inspect().unwrap()).is_empty());
    restored.close().unwrap();
    assert!(!storage.join("a1").exists());
}

#[test]
fn deleted_head_file_stays_deleted_in_the_initial_copy() {
    let (_temp, project, storage) = fixture();
    fs::remove_file(project.join("tracked.txt")).unwrap();
    git(&project, &["add", "-u"]);
    let workspace = Workspace::prepare(&project, &storage, "a1", Mode::Isolated).unwrap();
    assert!(!workspace.root().join("tracked.txt").exists());
    assert!(changed(&workspace.inspect().unwrap()).is_empty());
    workspace.close().unwrap();
}

#[test]
fn integration_requires_selected_paths_and_reports_overlap() {
    let (_temp, project, storage) = fixture();
    let workspace = Workspace::prepare(&project, &storage, "a1", Mode::Isolated).unwrap();
    fs::write(workspace.root().join("tracked.txt"), "child\n").unwrap();
    fs::write(workspace.root().join("new.txt"), "child new\n").unwrap();
    fs::write(project.join("tracked.txt"), "main\n").unwrap();
    let review = workspace.inspect().unwrap();
    assert_eq!(changed(&review), vec!["new.txt", "tracked.txt"]);
    let token = review.get("review_token").unwrap().as_str().unwrap();
    let result = workspace
        .integrate(&["new.txt".into(), "tracked.txt".into()], token)
        .unwrap();
    assert_eq!(result.get("applied").unwrap().as_array().unwrap().len(), 1);
    assert_eq!(
        result.get("conflicts").unwrap().as_array().unwrap().len(),
        1
    );
    assert_eq!(
        fs::read_to_string(project.join("tracked.txt")).unwrap(),
        "main\n"
    );
    assert_eq!(
        fs::read_to_string(project.join("new.txt")).unwrap(),
        "child new\n"
    );
    workspace.close().unwrap();
}

#[test]
fn isolated_workspace_accepts_submodules_and_nested_repositories() {
    let (temp, project, storage) = fixture();
    let (_other, library, _) = fixture();
    git(
        &project,
        &[
            "-c",
            "protocol.file.allow=always",
            "submodule",
            "add",
            "-q",
            &library.to_string_lossy(),
            "library",
        ],
    );
    git(
        &project,
        &[
            "-c",
            "user.name=Giovanni Jecha",
            "-c",
            "user.email=100841799+giovannijecha@users.noreply.github.com",
            "commit",
            "-qm",
            "submodule",
        ],
    );
    let nested = project.join("nested");
    fs::create_dir(&nested).unwrap();
    git(&nested, &["init", "-q"]);
    fs::write(nested.join("inner.txt"), "inner\n").unwrap();
    let workspace = Workspace::prepare(&project, &storage, "a1", Mode::Isolated).unwrap();
    fs::write(workspace.root().join("tracked.txt"), "child\n").unwrap();
    assert_eq!(changed(&workspace.inspect().unwrap()), vec!["tracked.txt"]);
    workspace.close().unwrap();
    drop(temp);
}

#[test]
fn a_failure_after_an_applied_path_reports_what_was_applied() {
    let (_temp, project, storage) = fixture();
    let workspace = Workspace::prepare(&project, &storage, "a1", Mode::Isolated).unwrap();
    fs::write(workspace.root().join("new.txt"), "child new\n").unwrap();
    fs::write(workspace.root().join("tracked.txt"), "child\n").unwrap();
    let review = workspace.inspect().unwrap();
    let token = review.get("review_token").unwrap().as_str().unwrap();
    // The saved baseline of the second path no longer matches its record.
    fs::write(workspace.storage.join("base/tracked.txt"), "tampered\n").unwrap();
    let result = workspace
        .integrate(&["new.txt".into(), "tracked.txt".into()], token)
        .unwrap();
    assert_eq!(
        result.get("applied").unwrap().as_array().unwrap(),
        &[Value::string("new.txt")]
    );
    let failed = result.get("failed").unwrap().as_array().unwrap();
    assert_eq!(failed[0].get("path"), Some(&Value::string("tracked.txt")));
    assert_eq!(
        fs::read_to_string(project.join("new.txt")).unwrap(),
        "child new\n"
    );
    assert_eq!(
        fs::read_to_string(project.join("tracked.txt")).unwrap(),
        "head\n"
    );
    workspace.close().unwrap();
}

#[test]
fn changed_since_review_is_rejected_and_ownership_is_checked() {
    let (_temp, project, storage) = fixture();
    let workspace = Workspace::prepare(&project, &storage, "a1", Mode::Isolated).unwrap();
    fs::write(workspace.root().join("tracked.txt"), "child\n").unwrap();
    let review = workspace.inspect().unwrap();
    fs::write(project.join("tracked.txt"), "late\n").unwrap();
    let token = review.get("review_token").unwrap().as_str().unwrap();
    assert!(workspace.integrate(&["tracked.txt".into()], token).is_err());
    assert_eq!(
        fs::read_to_string(project.join("tracked.txt")).unwrap(),
        "late\n"
    );
    let mut forged = workspace.value();
    if let Value::Object(fields) = &mut forged {
        fields.insert("project".into(), Value::string(storage.to_string_lossy()));
    }
    assert!(Workspace::from_value(&forged).is_err());
    workspace.close().unwrap();
}

#[test]
fn ignored_agent_instructions_and_text_evidence_are_available() {
    let (_temp, project, storage) = fixture();
    fs::write(project.join(".gitignore"), "AGENTS.md\n").unwrap();
    fs::write(project.join("AGENTS.md"), "Local instructions\n").unwrap();
    let workspace = Workspace::prepare(&project, &storage, "a1", Mode::Isolated).unwrap();
    assert_eq!(
        fs::read_to_string(workspace.root().join("AGENTS.md")).unwrap(),
        "Local instructions\n"
    );
    fs::write(workspace.root().join("tracked.txt"), "replacement\n").unwrap();
    let review = workspace.inspect().unwrap();
    let entry = review
        .get("changed")
        .unwrap()
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry.get("path").unwrap().as_str() == Some("tracked.txt"))
        .unwrap();
    assert_eq!(
        entry.get("evidence").unwrap().get("kind").unwrap().as_str(),
        Some("text_diff")
    );
    let page = workspace.inspect_file("tracked.txt", 0, 4).unwrap();
    assert_eq!(
        page.get("before").unwrap().get("content").unwrap().as_str(),
        Some("head")
    );
    assert_eq!(
        page.get("after").unwrap().get("content").unwrap().as_str(),
        Some("repl")
    );
    workspace.close().unwrap();
}

#[test]
fn failed_copy_cleans_owned_worktree_and_allows_retry() {
    let (_temp, project, storage) = fixture();
    // The copy fails after the worktree exists: the source cannot be read on
    // Windows, and Unix allows a name the workspace cannot represent.
    #[cfg(windows)]
    let blocker = {
        use std::os::windows::fs::OpenOptionsExt;
        fs::OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(project.join("tracked.txt"))
            .unwrap()
    };
    #[cfg(unix)]
    let blocker = {
        let path = project.join("bad\\name.txt");
        fs::write(&path, "x").unwrap();
        path
    };
    assert!(Workspace::prepare(&project, &storage, "a1", Mode::Isolated).is_err());
    assert!(!storage.join("a1").exists());
    #[cfg(windows)]
    drop(blocker);
    #[cfg(unix)]
    fs::remove_file(blocker).unwrap();
    let workspace = Workspace::prepare(&project, &storage, "a1", Mode::Isolated).unwrap();
    workspace.close().unwrap();
}

#[test]
fn modified_baseline_content_cannot_support_review_or_integration() {
    let (_temp, project, storage) = fixture();
    let workspace = Workspace::prepare(&project, &storage, "a1", Mode::Isolated).unwrap();
    fs::write(workspace.root().join("tracked.txt"), "child\n").unwrap();
    let token = workspace
        .inspect()
        .unwrap()
        .get("review_token")
        .unwrap()
        .as_str()
        .unwrap()
        .to_owned();
    fs::write(storage.join("a1/base/tracked.txt"), "forged\n").unwrap();
    assert!(workspace.inspect().is_err());
    assert!(
        workspace
            .integrate(&["tracked.txt".into()], &token)
            .is_err()
    );
    assert_eq!(
        fs::read_to_string(project.join("tracked.txt")).unwrap(),
        "head\n"
    );
    workspace.close().unwrap();
}

#[test]
fn interrupted_owned_preparation_is_recovered_before_reusing_id() {
    let (_temp, project, storage) = fixture();
    let workspace = Workspace::prepare(&project, &storage, "a1", Mode::Isolated).unwrap();
    assert!(workspace.root().join("tracked.txt").exists());
    let marker = storage.join("a1/baseline.json");
    let ready = fs::read_to_string(&marker).unwrap();
    assert!(ready.contains("\"ready\":true"));
    fs::write(&marker, ready.replace("\"ready\":true", "\"ready\":false")).unwrap();
    let recovered = Workspace::prepare(&project, &storage, "a1", Mode::Isolated).unwrap();
    assert_eq!(
        fs::read_to_string(recovered.root().join("tracked.txt")).unwrap(),
        "head\n"
    );
    assert!(changed(&recovered.inspect().unwrap()).is_empty());
    recovered.close().unwrap();
}

#[test]
fn completed_and_foreign_workspaces_are_preserved_on_id_collision() {
    let (_temp, project, storage) = fixture();
    let workspace = Workspace::prepare(&project, &storage, "a1", Mode::Isolated).unwrap();
    let marker = storage.join("a1/baseline.json");
    let ready = fs::read_to_string(&marker).unwrap();
    assert!(Workspace::prepare(&project, &storage, "a1", Mode::Isolated).is_err());
    assert!(workspace.root().join("tracked.txt").exists());
    fs::write(
        &marker,
        ready.replace("\"id\":\"a1\"", "\"id\":\"foreign\""),
    )
    .unwrap();
    assert!(Workspace::prepare(&project, &storage, "a1", Mode::Isolated).is_err());
    assert!(workspace.root().join("tracked.txt").exists());
    fs::write(&marker, ready).unwrap();
    workspace.close().unwrap();
}

#[test]
fn interrupted_worktree_with_missing_tree_can_retry_same_id() {
    let (_temp, project, storage) = fixture();
    let workspace = Workspace::prepare(&project, &storage, "a1", Mode::Isolated).unwrap();
    let marker = storage.join("a1/baseline.json");
    let ready = fs::read_to_string(&marker).unwrap();
    fs::write(&marker, ready.replace("\"ready\":true", "\"ready\":false")).unwrap();
    fs::remove_dir_all(workspace.root()).unwrap();
    let recovered = Workspace::prepare(&project, &storage, "a1", Mode::Isolated).unwrap();
    recovered.close().unwrap();
}

fn link_directory(target: &Path, link: &Path) {
    #[cfg(unix)]
    std::os::unix::fs::symlink(target, link).unwrap();
    #[cfg(windows)]
    {
        let created = Command::new("cmd.exe")
            .args(["/c", "mklink", "/J"])
            .arg(link.to_string_lossy().replace('/', "\\"))
            .arg(target.to_string_lossy().replace('/', "\\"))
            .output()
            .unwrap();
        assert!(created.status.success(), "{created:?}");
    }
}

#[test]
fn closing_removes_links_without_touching_their_targets() {
    let (temp, project, storage) = fixture();
    let outside = temp.path().join("outside");
    fs::create_dir(&outside).unwrap();
    fs::write(outside.join("canary.txt"), "keep\n").unwrap();
    let workspace = Workspace::prepare(&project, &storage, "a1", Mode::Isolated).unwrap();
    let nested = workspace.root().join("nested");
    fs::create_dir(&nested).unwrap();
    link_directory(&project, &workspace.root().join("project-link"));
    link_directory(&outside, &nested.join("outside-link"));
    workspace.close().unwrap();
    assert!(!storage.join("a1").exists());
    assert_eq!(
        fs::read_to_string(outside.join("canary.txt")).unwrap(),
        "keep\n"
    );
    assert_eq!(
        fs::read_to_string(project.join("tracked.txt")).unwrap(),
        "head\n"
    );
}
