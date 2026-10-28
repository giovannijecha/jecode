use super::*;
use crate::{effort::Effort, test_support::Directory};

#[test]
fn deleting_isolated_collaboration_keeps_owned_files_and_parent_record() {
    use crate::{
        collaboration::workspace::{Mode, Workspace},
        json::Value,
    };
    use std::process::Command;

    let (_home, project, store, mut document) = fixture();
    let git = |args: &[&str]| {
        let output = Command::new("git")
            .arg("-C")
            .arg(project.path())
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    };
    git(&["init", "-q"]);
    fs::write(project.path().join("file.txt"), "source\n").unwrap();
    git(&["add", "file.txt"]);
    git(&[
        "-c",
        "user.name=Giovanni Jecha",
        "-c",
        "user.email=100841799+giovannijecha@users.noreply.github.com",
        "commit",
        "-qm",
        "initial",
    ]);
    let retained = store.bucket.join("collaboration").join(&document.id);
    let workspace = Workspace::prepare(project.path(), &retained, "a1", Mode::Isolated).unwrap();
    let record = Value::object([
        ("id", Value::string("a1")),
        ("number", Value::number(1)),
        ("title", Value::string("Inspect")),
        ("state", Value::string("returned")),
        ("workspace", Value::string("isolated")),
        ("workspace_info", workspace.value()),
        (
            "directory",
            Value::string(workspace.root().to_string_lossy()),
        ),
        ("sent_version", Value::number(1)),
        ("received_version", Value::number(1)),
        (
            "report",
            Value::object([
                ("version", Value::number(1)),
                ("text", Value::string("Completed report")),
            ]),
        ),
    ]);
    document.events.push(Value::object([
        ("type", Value::string("collaboration")),
        ("sequence", Value::number(1)),
        ("kind", Value::string("returned")),
        ("records", Value::Array(vec![record])),
        ("after_message", Value::number(document.messages.len())),
    ]));
    let journal = store.journal_path(&document.id);
    let lease = store.acquire(&document.id).unwrap();
    lease.save(&document).unwrap();
    drop(lease);

    let report = store.delete(&document.id).unwrap();
    assert_eq!(
        report.retained_collaboration.as_deref(),
        Some(retained.as_path())
    );
    assert!(!journal.exists());
    assert!(workspace.root().is_dir());
    let manifest =
        crate::json::parse(&fs::read_to_string(retained.join("parent-1.json")).unwrap()).unwrap();
    assert_eq!(
        manifest.get("parent_session_id").and_then(Value::as_str),
        Some(document.id.as_str())
    );
    assert_eq!(
        manifest.get("working_directory").and_then(Value::as_str),
        Some(document.directory.to_str().unwrap())
    );
    let saved = &manifest.get("records").and_then(Value::as_array).unwrap()[0];
    assert_eq!(
        saved.get("received_version").and_then(Value::as_usize),
        Some(1)
    );
    assert_eq!(
        saved
            .get("report")
            .and_then(|report| report.get("text"))
            .and_then(Value::as_str),
        Some("Completed report")
    );
}

#[test]
fn conflicting_retained_record_keeps_parent_journal_intact() {
    let (_home, _project, store, document) = fixture();
    let retained = store.bucket.join("collaboration").join(&document.id);
    fs::create_dir_all(&retained).unwrap();
    fs::write(retained.join("child.txt"), "owned child data").unwrap();
    fs::write(retained.join("parent-0.json"), "foreign record").unwrap();
    let journal = store.journal_path(&document.id);
    let lease = store.acquire(&document.id).unwrap();
    lease.save(&document).unwrap();
    drop(lease);
    let before = fs::read(&journal).unwrap();

    let error = store.delete(&document.id).unwrap_err();
    assert!(error.contains("Retained collaborator record changed"));
    assert_eq!(fs::read(&journal).unwrap(), before);
    assert_eq!(
        fs::read_to_string(retained.join("child.txt")).unwrap(),
        "owned child data"
    );
}

fn fixture() -> (Directory, Directory, Store, Document) {
    let home = Directory::new();
    let project = Directory::new();
    let store = Store::new(home.path().to_path_buf(), project.path()).unwrap();
    let document = Document::new(
        project.path().to_path_buf(),
        "fixture/model".into(),
        Effort::Default,
    )
    .unwrap();
    let mut document = document;
    document.messages = vec![
        crate::json::Value::object([
            ("role", crate::json::Value::string("system")),
            ("content", crate::json::Value::string("fixture")),
        ]),
        crate::json::Value::object([
            ("role", crate::json::Value::string("user")),
            ("content", crate::json::Value::string("hello")),
        ]),
        crate::json::Value::object([
            ("role", crate::json::Value::string("assistant")),
            ("content", crate::json::Value::string("hi")),
        ]),
    ];
    (home, project, store, document)
}

#[test]
fn inactive_delete_holds_lease_and_preserves_lock_path() {
    let (_home, _project, store, document) = fixture();
    let lease = store.acquire(&document.id).unwrap();
    lease.save(&document).unwrap();
    let journal = store.journal_path(&document.id);
    let lock = store.bucket.join(format!("{}.lock", document.id));
    assert!(store.delete(&document.id).is_err());
    assert!(journal.exists());
    drop(lease);
    let report = store.delete(&document.id).unwrap();
    assert!(report.files >= 1);
    assert!(!journal.exists());
    assert!(lock.is_file());
    assert!(store.acquire(&document.id).is_ok());
}

#[test]
fn failed_output_root_removal_preserves_journal_and_allows_a_later_delete() {
    let (_home, project, store, document) = fixture();
    let lease = store.acquire(&document.id).unwrap();
    lease.save(&document).unwrap();
    let journal = store.journal_path(&document.id);
    let before = fs::read(&journal).unwrap();
    let root = store.output_directory();
    let output = crate::output::Store::for_session(
        root.clone(),
        project.path().to_path_buf(),
        &document.id,
        crate::redact::Redactor::empty(),
    )
    .unwrap();
    drop(output.create().unwrap());
    let removal = store.removal(&document).unwrap();
    // A late file makes the final directory removal fail after prepared entries.
    fs::write(root.join(&document.id).join("late-file"), "late").unwrap();
    let error = removal.remove().unwrap_err();
    assert!(error.contains("Output ownership was restored"), "{error}");
    assert_eq!(fs::read(&journal).unwrap(), before);
    drop(lease);
    store.delete(&document.id).unwrap();
    assert!(!journal.exists());
    assert!(!root.join(&document.id).exists());
}

#[test]
fn missing_invalid_and_unrelated_ids_leave_files_untouched() {
    let (_home, _project, store, document) = fixture();
    let lease = store.acquire(&document.id).unwrap();
    lease.save(&document).unwrap();
    drop(lease);
    let journal = store.journal_path(&document.id);
    assert!(store.delete("../config").is_err());
    assert!(store.delete("999-1-1").is_err());
    assert!(journal.exists());
    let neighboring = store.bucket.join(format!("{}0.summary.json", document.id));
    fs::write(&neighboring, "other").unwrap();
    let malformed = store
        .bucket
        .join(format!("{}.json.damaged---", document.id));
    fs::write(&malformed, "other").unwrap();
    store.delete(&document.id).unwrap();
    assert_eq!(fs::read_to_string(neighboring).unwrap(), "other");
    assert_eq!(fs::read_to_string(malformed).unwrap(), "other");
}

#[test]
fn invalid_bucket_is_rejected_before_creating_a_lock() {
    let (_home, _project, store, document) = fixture();
    fs::create_dir_all(store.bucket.parent().unwrap()).unwrap();
    fs::write(&store.bucket, "not a directory").unwrap();
    assert!(store.delete(&document.id).is_err());
    assert_eq!(
        fs::read_to_string(&store.bucket).unwrap(),
        "not a directory"
    );
}

#[cfg(any(unix, windows))]
#[test]
fn linked_bucket_is_rejected_before_writing_to_its_target() {
    let (_home, _project, store, document) = fixture();
    let target = Directory::new();
    let protected = target.path().join("outside.txt");
    fs::write(&protected, "outside").unwrap();
    fs::create_dir_all(store.bucket.parent().unwrap()).unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(target.path(), &store.bucket).unwrap();
    #[cfg(windows)]
    {
        let junction = std::process::Command::new("cmd.exe")
            .args(["/c", "mklink", "/J"])
            .arg(store.bucket.to_string_lossy().replace('/', "\\"))
            .arg(target.path().to_string_lossy().replace('/', "\\"))
            .output()
            .unwrap();
        assert!(junction.status.success(), "{junction:?}");
    }
    assert!(store.delete(&document.id).is_err());
    assert!(!target.path().join(format!("{}.lock", document.id)).exists());
    assert_eq!(fs::read_to_string(protected).unwrap(), "outside");
    #[cfg(windows)]
    fs::remove_dir(&store.bucket).unwrap();
    #[cfg(unix)]
    fs::remove_file(&store.bucket).unwrap();
}

#[test]
fn known_sidecars_are_removed_with_the_journal() {
    let (_home, _project, store, document) = fixture();
    let lease = store.acquire(&document.id).unwrap();
    lease.save(&document).unwrap();
    drop(lease);
    let journal = store.journal_path(&document.id);
    let sidecars = [
        format!("{}.jsonl.damaged-1-2-3", document.id),
        format!("{}.summary.json", document.id),
        format!("{}.summary.tmp-1-2-3", document.id),
        format!("{}.tmp-1-2-3", document.id),
    ];
    for name in &sidecars {
        fs::write(store.bucket.join(name), "owned").unwrap();
    }
    let report = store.delete(&document.id).unwrap();
    assert_eq!(report.files, 5);
    assert!(!journal.exists());
    for name in &sidecars {
        assert!(!store.bucket.join(name).exists());
    }
}

#[test]
fn deleting_a_migrated_session_removes_its_original_copies() {
    let (_home, _project, store, document) = fixture();
    let lease = store.acquire(&document.id).unwrap();
    lease.save(&document).unwrap();
    drop(lease);
    let originals = [
        format!("{}.json", document.id),
        format!("{}.json.bak", document.id),
        format!("{}.json.damaged-1-2-3", document.id),
    ];
    for name in &originals {
        fs::write(store.bucket.join(name), document.value().encode()).unwrap();
    }
    store.delete(&document.id).unwrap();
    assert!(!store.journal_path(&document.id).exists());
    for name in originals {
        assert!(!store.bucket.join(&name).exists(), "Retained {name}");
    }
}

#[test]
fn unverified_original_copies_block_deletion_before_removing_the_journal() {
    let (_home, _project, store, document) = fixture();
    let lease = store.acquire(&document.id).unwrap();
    lease.save(&document).unwrap();
    drop(lease);
    let original = store.bucket.join(format!("{}.json", document.id));
    let mut foreign = document.value();
    if let crate::json::Value::Object(ref mut fields) = foreign {
        fields.insert("id".into(), crate::json::Value::string("999-1-1"));
    }
    for text in [foreign.encode(), "{broken".into()] {
        fs::write(&original, &text).unwrap();
        assert!(store.delete(&document.id).is_err());
        assert!(store.journal_path(&document.id).exists());
        assert_eq!(fs::read_to_string(&original).unwrap(), text);
    }
}

#[test]
fn corrupt_or_foreign_durable_record_is_rejected_before_cleanup() {
    let (_home, _project, store, document) = fixture();
    let lease = store.acquire(&document.id).unwrap();
    lease.save(&document).unwrap();
    drop(lease);
    let journal = store.journal_path(&document.id);
    let sidecar = store.bucket.join(format!("{}.summary.json", document.id));
    for record in [
        "{broken\n",
        "{\"format\":\"jecode.session.journal\",\"format_version\":99}\n",
    ] {
        fs::write(&journal, record).unwrap();
        fs::write(&sidecar, "cache").unwrap();
        assert!(store.delete(&document.id).is_err());
        assert_eq!(fs::read_to_string(&journal).unwrap(), record);
        assert!(sidecar.exists());
    }
}

#[test]
fn special_sidecar_blocks_all_cleanup_and_ancillary_failure_keeps_journal() {
    let (_home, _project, store, document) = fixture();
    let lease = store.acquire(&document.id).unwrap();
    lease.save(&document).unwrap();
    let journal = store.journal_path(&document.id);
    let sidecar = store.bucket.join(format!("{}.summary.json", document.id));
    if sidecar.exists() {
        fs::remove_file(&sidecar).unwrap();
    }
    fs::create_dir(&sidecar).unwrap();
    assert!(store.removal(&document).is_err());
    assert!(journal.exists());
    fs::remove_dir(&sidecar).unwrap();
    fs::write(&sidecar, "cache").unwrap();
    let removal = store.removal(&document).unwrap();
    fs::remove_file(&sidecar).unwrap();
    fs::create_dir(&sidecar).unwrap();
    assert!(removal.remove().is_err());
    assert!(journal.exists());
    drop(lease);
}

#[test]
fn fresh_session_can_remove_an_owned_temporary_tree() {
    let (_home, _project, store, document) = fixture();
    let _lease = store.acquire(&document.id).unwrap();
    let area = store.temporary_area(&document.id).unwrap();
    area.ensure().unwrap();
    let owned = area.path().join("orphan.txt");
    fs::write(&owned, "orphan").unwrap();
    let report = store.removal(&document).unwrap().remove().unwrap();
    assert_eq!(report.files, 2);
    assert_eq!(report.directories, 1);
    assert!(!area.path().exists());
}
