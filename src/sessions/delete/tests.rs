use super::*;
use crate::{effort::Effort, test_support::Directory};

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
fn legacy_recovery_and_known_sidecars_are_removed() {
    let (_home, _project, store, document) = fixture();
    let lease = store.acquire(&document.id).unwrap();
    lease.save_legacy(&document).unwrap();
    lease.save_legacy(&document).unwrap();
    drop(lease);
    let primary = store.bucket.join(format!("{}.json", document.id));
    let backup = store.bucket.join(format!("{}.json.bak", document.id));
    fs::write(&primary, "{broken primary").unwrap();
    let sidecars = [
        format!("{}.json.damaged-1-2-3", document.id),
        format!("{}.jsonl.damaged-1-2-3", document.id),
        format!("{}.summary.json", document.id),
        format!("{}.summary.tmp-1-2-3", document.id),
        format!("{}.tmp-1-2-3", document.id),
    ];
    for name in &sidecars {
        fs::write(store.bucket.join(name), "owned").unwrap();
    }
    let report = store.delete(&document.id).unwrap();
    assert_eq!(report.files, 7);
    assert!(!primary.exists());
    assert!(!backup.exists());
    for name in &sidecars {
        assert!(!store.bucket.join(name).exists());
    }
}

#[test]
fn corrupt_or_foreign_durable_record_is_rejected_before_cleanup() {
    let (_home, _project, store, document) = fixture();
    let lease = store.acquire(&document.id).unwrap();
    lease.save(&document).unwrap();
    drop(lease);
    let journal = store.journal_path(&document.id);
    let primary = store.bucket.join(format!("{}.json", document.id));
    fs::write(&primary, "{broken").unwrap();
    let sidecar = store.bucket.join(format!("{}.summary.json", document.id));
    fs::write(&sidecar, "cache").unwrap();
    assert!(store.delete(&document.id).is_err());
    assert!(journal.exists());
    assert!(sidecar.exists());
    let mut foreign = document.value();
    if let crate::json::Value::Object(ref mut fields) = foreign {
        fields.insert("format_version".into(), crate::json::Value::number(99));
    }
    fs::write(&primary, foreign.encode()).unwrap();
    assert!(store.delete(&document.id).is_err());
    assert!(journal.exists());
    assert!(sidecar.exists());
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
