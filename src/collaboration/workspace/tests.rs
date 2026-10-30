use super::*;
use crate::test_support::{Directory, link_directory};
use std::process::Command;

fn fixture() -> (Directory, PathBuf, PathBuf) {
    let temp = Directory::new();
    let project = temp.path().join("project");
    let storage = temp.path().join("storage");
    fs::create_dir(&project).unwrap();
    fs::write(project.join("tracked.txt"), "head\n").unwrap();
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
fn isolated_windows_long_path_is_owned_and_cleans_up() {
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
                assert!(!error.contains("Git"), "{error}");
                assert_eq!(
                    fs::read_to_string(project.join("tracked.txt")).unwrap(),
                    "head\n"
                );
                assert!(!storage.join("a1").exists());
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
    }
}

#[test]
fn working_and_new_files_are_copied() {
    let (_temp, project, storage) = fixture();
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
fn absent_project_file_stays_absent_in_the_initial_copy() {
    let (_temp, project, storage) = fixture();
    fs::remove_file(project.join("tracked.txt")).unwrap();
    let workspace = Workspace::prepare(&project, &storage, "a1", Mode::Isolated).unwrap();
    assert!(!workspace.root().join("tracked.txt").exists());
    assert!(changed(&workspace.inspect().unwrap()).is_empty());
    workspace.close().unwrap();
}

#[test]
fn isolated_copy_integrates_and_closes_without_git_on_path() {
    if std::env::var_os("JECODE_TEST_NO_GIT_PATH").is_none() {
        let output = Command::new(std::env::current_exe().unwrap())
            .arg("--exact")
            .arg("collaboration::workspace::tests::isolated_copy_integrates_and_closes_without_git_on_path")
            .arg("--nocapture")
            .env("PATH", "")
            .env("JECODE_TEST_NO_GIT_PATH", "1")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stdout)
        );
        return;
    }
    let (_temp, project, storage) = fixture();
    assert!(!project.join(".git").exists());
    let workspace = Workspace::prepare(&project, &storage, "a1", Mode::Isolated).unwrap();
    fs::write(workspace.root().join("tracked.txt"), "child\n").unwrap();
    let review = workspace.inspect().unwrap();
    assert_eq!(changed(&review), vec!["tracked.txt"]);
    let token = review.get("review_token").unwrap().as_str().unwrap();
    let result = workspace.integrate(&["tracked.txt".into()], token).unwrap();
    assert_eq!(
        result.get("applied").unwrap().as_array().unwrap(),
        &[Value::string("tracked.txt")]
    );
    assert_eq!(
        fs::read_to_string(project.join("tracked.txt")).unwrap(),
        "child\n"
    );
    workspace.close().unwrap();
    assert!(!storage.join("a1").exists());
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
fn isolated_workspace_skips_service_trees_and_copies_nested_files() {
    let (_temp, project, storage) = fixture();
    let nested = project.join("nested");
    fs::create_dir(&nested).unwrap();
    fs::write(nested.join("inner.txt"), "inner\n").unwrap();
    fs::create_dir_all(nested.join("target")).unwrap();
    fs::write(nested.join("target/data.txt"), "source data\n").unwrap();
    for name in [".git", ".jecode", "target", ".tmp"] {
        fs::create_dir(project.join(name)).unwrap();
        fs::write(project.join(name).join("secret.txt"), name).unwrap();
    }
    let workspace = Workspace::prepare(&project, &storage, "a1", Mode::Isolated).unwrap();
    assert_eq!(
        fs::read_to_string(workspace.root().join("nested/inner.txt")).unwrap(),
        "inner\n"
    );
    assert_eq!(
        fs::read_to_string(workspace.root().join("nested/target/data.txt")).unwrap(),
        "source data\n"
    );
    for name in [".git", ".jecode", "target", ".tmp"] {
        assert!(!workspace.root().join(name).exists());
    }
    fs::write(workspace.root().join("tracked.txt"), "child\n").unwrap();
    assert_eq!(changed(&workspace.inspect().unwrap()), vec!["tracked.txt"]);
    workspace.close().unwrap();
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
fn unowned_complete_copy_is_reclaimed_and_foreign_entries_are_preserved() {
    let (_temp, project, storage) = fixture();
    let workspace = Workspace::prepare(&project, &storage, "a1", Mode::Isolated).unwrap();
    let marker = storage.join("a1/baseline.json");
    let ready = fs::read_to_string(&marker).unwrap();
    // Preparation cannot tell whether a saved record owns a complete copy.
    assert!(Workspace::prepare(&project, &storage, "a1", Mode::Isolated).is_err());
    assert!(workspace.root().join("tracked.txt").exists());
    fs::write(
        &marker,
        ready.replace("\"id\":\"a1\"", "\"id\":\"foreign\""),
    )
    .unwrap();
    assert!(Workspace::prepare(&project, &storage, "a1", Mode::Isolated).is_err());
    assert!(Workspace::reclaim(&project, &storage, "a1").is_err());
    assert!(workspace.root().join("tracked.txt").exists());
    fs::write(&marker, ready).unwrap();
    // A copy no saved record owns, as after a stop before its `created`
    // receipt, is removed from owned storage.
    Workspace::reclaim(&project, &storage, "a1").unwrap();
    assert!(!storage.join("a1").exists());
    // A stop before or while writing the marker leaves an empty directory or
    // a staging file; anything else without a marker stays.
    fs::create_dir(storage.join("a2")).unwrap();
    Workspace::reclaim(&project, &storage, "a2").unwrap();
    fs::create_dir(storage.join("a3")).unwrap();
    fs::write(
        crate::disk::temporary_beside(&storage.join("a3/baseline.json")),
        "{",
    )
    .unwrap();
    Workspace::reclaim(&project, &storage, "a3").unwrap();
    assert!(!storage.join("a2").exists() && !storage.join("a3").exists());
    fs::create_dir(storage.join("a4")).unwrap();
    fs::write(storage.join("a4/notes.txt"), "keep\n").unwrap();
    assert!(Workspace::reclaim(&project, &storage, "a4").is_err());
    assert_eq!(
        fs::read_to_string(storage.join("a4/notes.txt")).unwrap(),
        "keep\n"
    );
    let reused = Workspace::prepare(&project, &storage, "a1", Mode::Isolated).unwrap();
    reused.close().unwrap();
}

#[test]
fn interrupted_copy_with_missing_tree_can_retry_same_id() {
    let (_temp, project, storage) = fixture();
    let workspace = Workspace::prepare(&project, &storage, "a1", Mode::Isolated).unwrap();
    let marker = storage.join("a1/baseline.json");
    let ready = fs::read_to_string(&marker).unwrap();
    fs::write(&marker, ready.replace("\"ready\":true", "\"ready\":false")).unwrap();
    fs::remove_dir_all(workspace.root()).unwrap();
    let recovered = Workspace::prepare(&project, &storage, "a1", Mode::Isolated).unwrap();
    recovered.close().unwrap();
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

mod copying;
mod directories;
