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
    fs::remove_file(project.join("tracked.txt")).unwrap();
    fs::create_dir(project.join("tracked.txt")).unwrap();
    fs::write(project.join("tracked.txt").join("nested.txt"), "nested").unwrap();
    assert!(Workspace::prepare(&project, &storage, "a1", Mode::Isolated).is_err());
    assert!(!storage.join("a1").exists());
    fs::remove_dir_all(project.join("tracked.txt")).unwrap();
    fs::write(project.join("tracked.txt"), "restored\n").unwrap();
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
