use super::*;

#[test]
fn same_named_root_files_and_empty_directories_are_copied() {
    let (_temp, project, storage) = fixture();
    for name in ["target", ".tmp"] {
        fs::write(project.join(name), name).unwrap();
    }
    fs::create_dir(project.join("empty")).unwrap();
    let workspace = Workspace::prepare(&project, &storage, "a1", Mode::Isolated).unwrap();
    assert_eq!(
        fs::read_to_string(workspace.root().join("target")).unwrap(),
        "target"
    );
    assert_eq!(
        fs::read_to_string(workspace.root().join(".tmp")).unwrap(),
        ".tmp"
    );
    assert!(workspace.root().join("empty").is_dir());
    assert!(changed(&workspace.inspect().unwrap()).is_empty());
    workspace.close().unwrap();
}

#[test]
fn older_directory_baseline_without_mode_restores_from_owned_copy() {
    let (_temp, project, storage) = fixture();
    fs::create_dir(project.join("empty")).unwrap();
    let workspace = Workspace::prepare(&project, &storage, "a1", Mode::Isolated).unwrap();
    let marker = workspace.storage.join("baseline.json");
    let mut value = crate::json::parse(&fs::read_to_string(&marker).unwrap()).unwrap();
    let Value::Object(fields) = &mut value else {
        panic!("baseline root")
    };
    let Some(Value::Object(entries)) = fields.get_mut("entries") else {
        panic!("entries")
    };
    let Some(Value::Object(directory)) = entries.get_mut("empty") else {
        panic!("empty")
    };
    directory.remove("mode");
    fs::write(&marker, value.encode()).unwrap();
    let restored = Workspace::from_value(&workspace.value()).unwrap();
    assert!(changed(&restored.inspect().unwrap()).is_empty());
    restored.close().unwrap();
}

#[test]
fn empty_directory_creation_and_deletion_integrate_in_dependency_order() {
    let (_temp, project, storage) = fixture();
    fs::create_dir(project.join("old")).unwrap();
    let workspace = Workspace::prepare(&project, &storage, "a1", Mode::Isolated).unwrap();
    fs::remove_dir(workspace.root().join("old")).unwrap();
    fs::create_dir(workspace.root().join("new")).unwrap();
    fs::create_dir(workspace.root().join("new/empty")).unwrap();
    fs::write(workspace.root().join("new/file.txt"), "from child\n").unwrap();
    let review = workspace.inspect().unwrap();
    assert_eq!(
        changed(&review),
        vec!["new", "new/empty", "new/file.txt", "old"]
    );
    let paths = changed(&review);
    let token = review.get("review_token").unwrap().as_str().unwrap();
    let result = workspace.integrate(&paths, token).unwrap();
    assert_eq!(result.get("applied").unwrap().as_array().unwrap().len(), 4);
    assert!(project.join("new/empty").is_dir());
    assert_eq!(
        fs::read_to_string(project.join("new/file.txt")).unwrap(),
        "from child\n"
    );
    assert!(!project.join("old").exists());
    workspace.close().unwrap();
}

#[test]
fn nested_directory_deletions_follow_file_deletions() {
    let (_temp, project, storage) = fixture();
    fs::create_dir_all(project.join("old/deep")).unwrap();
    fs::write(project.join("old/deep/file.txt"), "source\n").unwrap();
    let workspace = Workspace::prepare(&project, &storage, "a1", Mode::Isolated).unwrap();
    fs::remove_file(workspace.root().join("old/deep/file.txt")).unwrap();
    fs::remove_dir(workspace.root().join("old/deep")).unwrap();
    fs::remove_dir(workspace.root().join("old")).unwrap();
    let review = workspace.inspect().unwrap();
    let token = review.get("review_token").unwrap().as_str().unwrap();
    let result = workspace.integrate(&changed(&review), token).unwrap();
    assert_eq!(result.get("applied").unwrap().as_array().unwrap().len(), 3);
    assert!(!project.join("old").exists());
    workspace.close().unwrap();
}

#[test]
fn directory_deletion_does_not_remove_concurrent_project_content() {
    let (_temp, project, storage) = fixture();
    fs::create_dir(project.join("old")).unwrap();
    let workspace = Workspace::prepare(&project, &storage, "a1", Mode::Isolated).unwrap();
    fs::remove_dir(workspace.root().join("old")).unwrap();
    let review = workspace.inspect().unwrap();
    fs::write(project.join("old/canary.txt"), "keep\n").unwrap();
    let token = review.get("review_token").unwrap().as_str().unwrap();
    let result = workspace.integrate(&["old".into()], token).unwrap();
    assert!(
        result
            .get("applied")
            .unwrap()
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert_eq!(result.get("failed").unwrap().as_array().unwrap().len(), 1);
    assert_eq!(
        fs::read_to_string(project.join("old/canary.txt")).unwrap(),
        "keep\n"
    );
    workspace.close().unwrap();
}

#[cfg(unix)]
#[test]
fn new_directory_mode_is_applied_after_child_files() {
    use std::os::unix::fs::PermissionsExt;
    let (_temp, project, storage) = fixture();
    let workspace = Workspace::prepare(&project, &storage, "a1", Mode::Isolated).unwrap();
    let child_dir = workspace.root().join("locked");
    fs::create_dir(&child_dir).unwrap();
    fs::write(child_dir.join("file.txt"), "child\n").unwrap();
    fs::set_permissions(&child_dir, fs::Permissions::from_mode(0o500)).unwrap();
    let review = workspace.inspect().unwrap();
    let token = review.get("review_token").unwrap().as_str().unwrap();
    let result = workspace
        .integrate(&["locked".into(), "locked/file.txt".into()], token)
        .unwrap();
    assert_eq!(result.get("applied").unwrap().as_array().unwrap().len(), 2);
    assert_eq!(
        fs::read_to_string(project.join("locked/file.txt")).unwrap(),
        "child\n"
    );
    assert_eq!(
        fs::metadata(project.join("locked"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o500
    );
    fs::set_permissions(&child_dir, fs::Permissions::from_mode(0o700)).unwrap();
    fs::set_permissions(project.join("locked"), fs::Permissions::from_mode(0o700)).unwrap();
    workspace.close().unwrap();
}
