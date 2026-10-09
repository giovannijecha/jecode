use super::*;
use crate::{effort::Effort, json::Value, test_support::Directory};
use std::path::Path;

fn document(project: &Path) -> crate::sessions::Document {
    crate::sessions::Document::new(
        project.to_path_buf(),
        "fixture/model".into(),
        Effort::Default,
    )
    .unwrap()
}

#[test]
fn fragmented_credentials_are_masked_in_durable_output() {
    let directory = Directory::new();
    let store = Store::new(
        directory.path().join("logs"),
        Redactor::new("test-secret-credential".into()),
    );
    let mut logs = store.create().unwrap();
    let original = "before test-secret-credential after crab\n";
    for byte in original.as_bytes() {
        logs.stdout.write(&[*byte]).unwrap();
    }
    logs.stdout.finish().unwrap();
    assert_eq!(
        fs::read_to_string(store.resolve(&logs.stdout_ref).unwrap()).unwrap(),
        "before [redacted] after crab\n"
    );
    assert!(store.resolve("output:../../config:stdout").is_err());
}

#[test]
fn owned_outputs_are_lazy_and_cross_session_references_survive_reconfiguration() {
    let home = Directory::new();
    let project = Directory::new();
    let root = home.path().join("outputs");
    let first = document(project.path());
    let second = document(project.path());
    let first_store = Store::for_session(
        root.clone(),
        project.path().to_path_buf(),
        &first.id,
        Redactor::empty(),
    )
    .unwrap();
    assert!(!root.exists());
    let mut logs = first_store.create().unwrap();
    logs.stdout.write(b"first session").unwrap();
    logs.stdout.finish().unwrap();
    let reference = logs.stdout_ref.clone();
    assert!(reference.starts_with(&format!("output:{}:", first.id)));
    assert!(root.join(&first.id).join(".jecode-output.json").is_file());
    drop(logs);

    let second_store = Store::for_session(
        root.clone(),
        project.path().to_path_buf(),
        &second.id,
        Redactor::empty(),
    )
    .unwrap();
    assert_eq!(
        fs::read_to_string(second_store.resolve(&reference).unwrap()).unwrap(),
        "first session"
    );
    let mut logs = second_store.create().unwrap();
    logs.stderr.write(b"second session").unwrap();
    logs.stderr.finish().unwrap();
    assert_eq!(
        fs::read_to_string(first_store.resolve(&logs.stderr_ref).unwrap()).unwrap(),
        "second session"
    );
    for reference in [
        "output:../../escape:1:stdout",
        "output:1:../../escape:stderr",
        "output:1:2:stdout:extra",
        "output:1:2:other",
    ] {
        assert!(second_store.resolve(reference).is_err(), "{reference}");
    }
}

#[test]
fn removal_cleans_orphans_and_keeps_legacy_flat_files() {
    let home = Directory::new();
    let project = Directory::new();
    let root = home.path().join("outputs");
    let mut selected = document(project.path());
    let store = Store::for_session(
        root.clone(),
        project.path().to_path_buf(),
        &selected.id,
        Redactor::empty(),
    )
    .unwrap();
    let logs = store.create().unwrap();
    let owned = root.join(&selected.id);
    drop(logs);
    fs::write(owned.join("orphan.stdout"), b"unreferenced").unwrap();
    fs::create_dir(owned.join("nested")).unwrap();
    fs::write(owned.join("nested").join("extra"), b"extra").unwrap();
    fs::write(root.join("8-8-8.stdout"), b"shared legacy").unwrap();
    selected.messages.push(Value::object([
        ("role", Value::string("tool")),
        (
            "content",
            Value::string(
                Value::object([
                    ("stdout_file", Value::string("output:8-8-8:stdout")),
                    ("stderr_file", Value::string("output:8-8-8:stdout")),
                ])
                .encode(),
            ),
        ),
    ]));
    let prepared = prepare_removal(&root, project.path(), &selected.id, &selected).unwrap();
    let removed = prepared.remove().unwrap();
    assert_eq!(removed.files, 5); // two streams, orphan, nested file, marker
    assert_eq!(removed.directories, 2);
    assert_eq!(removed.legacy_references, 1);
    assert!(!owned.exists());
    assert_eq!(
        fs::read(root.join("8-8-8.stdout")).unwrap(),
        b"shared legacy"
    );
    assert_eq!(
        fs::read(store.resolve("output:8-8-8:stdout").unwrap()).unwrap(),
        b"shared legacy"
    );
}

#[test]
fn absent_output_directory_is_not_created_during_removal() {
    let home = Directory::new();
    let project = Directory::new();
    let root = home.path().join("missing");
    let selected = document(project.path());
    let removed = prepare_removal(&root, project.path(), &selected.id, &selected)
        .unwrap()
        .remove()
        .unwrap();
    assert_eq!(removed.files, 0);
    assert_eq!(removed.directories, 0);
    assert!(!root.exists());
}

#[test]
fn foreign_or_missing_marker_preserves_existing_output() {
    let home = Directory::new();
    let project = Directory::new();
    let root = home.path().join("outputs");
    let selected = document(project.path());
    let store = Store::for_session(
        root.clone(),
        project.path().to_path_buf(),
        &selected.id,
        Redactor::empty(),
    )
    .unwrap();
    let logs = store.create().unwrap();
    drop(logs);
    let owned = root.join(&selected.id);
    let marker = owned.join(".jecode-output.json");
    let original = fs::read(&marker).unwrap();
    let foreign = Directory::new();
    fs::write(
        &marker,
        Value::object([
            ("format", Value::string("jecode.output")),
            ("format_version", Value::number(1)),
            ("directory", Value::string(foreign.path().to_string_lossy())),
            ("session_id", Value::string(&selected.id)),
        ])
        .encode(),
    )
    .unwrap();
    assert!(prepare_removal(&root, project.path(), &selected.id, &selected).is_err());
    assert!(store.create().is_err());
    assert!(owned.join(".jecode-output.json").exists());
    fs::write(&marker, b"{}").unwrap();
    assert!(prepare_removal(&root, project.path(), &selected.id, &selected).is_err());
    fs::write(&marker, original).unwrap();
    fs::remove_file(&marker).unwrap();
    assert!(prepare_removal(&root, project.path(), &selected.id, &selected).is_err());
    assert!(owned.exists());
}

#[cfg(windows)]
#[test]
fn linked_output_entry_blocks_removal_and_resolution_when_symlinks_are_available() {
    use std::os::windows::fs::symlink_file;
    let home = Directory::new();
    let project = Directory::new();
    let root = home.path().join("outputs");
    let selected = document(project.path());
    let store = Store::for_session(
        root.clone(),
        project.path().to_path_buf(),
        &selected.id,
        Redactor::empty(),
    )
    .unwrap();
    let logs = store.create().unwrap();
    let target = store.resolve(&logs.stdout_ref).unwrap();
    drop(logs);
    let link = root.join(&selected.id).join("9-9-9.stdout");
    if symlink_file(&target, &link).is_err() {
        return; // Windows requires Developer Mode or the symlink privilege.
    }
    assert!(
        store
            .resolve(&format!("output:{}:9-9-9:stdout", selected.id))
            .is_err()
    );
    assert!(prepare_removal(&root, project.path(), &selected.id, &selected).is_err());
    assert!(target.exists());
}

#[cfg(unix)]
#[test]
fn linked_output_entry_blocks_removal_and_resolution() {
    use std::os::unix::fs::symlink;
    let home = Directory::new();
    let project = Directory::new();
    let root = home.path().join("outputs");
    let selected = document(project.path());
    let store = Store::for_session(
        root.clone(),
        project.path().to_path_buf(),
        &selected.id,
        Redactor::empty(),
    )
    .unwrap();
    let logs = store.create().unwrap();
    let target = store.resolve(&logs.stdout_ref).unwrap();
    drop(logs);
    let link = root.join(&selected.id).join("9-9-9.stdout");
    symlink(&target, &link).unwrap();
    assert!(
        store
            .resolve(&format!("output:{}:9-9-9:stdout", selected.id))
            .is_err()
    );
    assert!(prepare_removal(&root, project.path(), &selected.id, &selected).is_err());
    assert!(target.exists());
}

#[cfg(unix)]
#[test]
fn failed_final_directory_removal_restores_owner_for_retry() {
    let home = Directory::new();
    let project = Directory::new();
    let root = home.path().join("outputs");
    let selected = document(project.path());
    let store = Store::for_session(
        root.clone(),
        project.path().to_path_buf(),
        &selected.id,
        Redactor::empty(),
    )
    .unwrap();
    drop(store.create().unwrap());
    let removal = prepare_removal(&root, project.path(), &selected.id, &selected).unwrap();
    let owned = root.join(&selected.id);
    fs::write(owned.join("late-file"), b"late").unwrap();
    let error = removal.remove().err().unwrap();
    assert!(error.contains("Output ownership was restored"), "{error}");
    assert!(owned.join(".jecode-output.json").is_file());
    let retry = prepare_removal(&root, project.path(), &selected.id, &selected).unwrap();
    let removed = retry.remove().unwrap();
    assert_eq!(removed.files, 2); // late file and restored marker
    assert_eq!(removed.directories, 1);
    assert!(!owned.exists());
}

#[cfg(unix)]
#[test]
fn parent_permissions_failure_retains_owner_for_retry() {
    use std::os::unix::fs::PermissionsExt;

    let home = Directory::new();
    let project = Directory::new();
    let root = home.path().join("outputs");
    let selected = document(project.path());
    let store = Store::for_session(
        root.clone(),
        project.path().to_path_buf(),
        &selected.id,
        Redactor::empty(),
    )
    .unwrap();
    drop(store.create().unwrap());
    let removal = prepare_removal(&root, project.path(), &selected.id, &selected).unwrap();
    let previous = fs::metadata(&root).unwrap().permissions();
    fs::set_permissions(&root, fs::Permissions::from_mode(0o500)).unwrap();
    let result = removal.remove();
    fs::set_permissions(&root, previous).unwrap();
    let Err(error) = result else {
        return; // A privileged test process may bypass directory permissions.
    };
    assert!(error.contains("Output ownership was restored"), "{error}");
    assert!(
        root.join(&selected.id)
            .join(".jecode-output.json")
            .is_file()
    );
    prepare_removal(&root, project.path(), &selected.id, &selected)
        .unwrap()
        .remove()
        .unwrap();
    assert!(!root.join(&selected.id).exists());
}
