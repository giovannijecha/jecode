//! The session folder layout: what each session owns, when folders appear,
//! and how files of the earlier flat layout are reported and left unchanged.

use super::*;
use crate::{effort::Effort, json::Value, redact::Redactor, test_support::Directory};
use std::fs;

fn message(role: &str, text: &str) -> Value {
    Value::object([
        ("role", Value::string(role)),
        ("content", Value::string(text)),
    ])
}

fn document(directory: &Directory) -> Document {
    let mut document = Document::new(
        directory.path().to_path_buf(),
        "fixture/model".into(),
        Effort::High,
    )
    .unwrap();
    document.messages = vec![
        message("system", "fixture system"),
        message("user", "First request"),
        message("assistant", "Answer"),
    ];
    document
}

fn store(home: &Directory, directory: &Directory) -> Store {
    Store::new(home.path().to_path_buf(), directory.path()).unwrap()
}

fn names(path: &std::path::Path) -> Vec<String> {
    let mut names = fs::read_dir(path)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect::<Vec<_>>();
    names.sort();
    names
}

#[test]
fn each_session_owns_one_folder_created_only_when_used() {
    let home = Directory::new();
    let a = Directory::new();
    let b = Directory::new();
    let store_a = store(&home, &a);
    let store_b = store(&home, &b);
    let doc = document(&a);
    let sessions = home.path().join("sessions");
    let folder = store_a.session_directory(&doc.id);
    assert_eq!(store_a.bucket().parent(), Some(sessions.as_path()));
    assert_eq!(folder, store_a.bucket().join(&doc.id));
    assert_ne!(store_a.bucket(), store_b.bucket());
    assert_eq!(
        store_a.attachments().directory(),
        store_a.bucket().join("attachments")
    );
    assert_eq!(
        store_a.collaborator_home(&doc.id),
        folder.join("collaboration").join("child-home")
    );

    // Preparing an area and an output store creates nothing.
    let area = store_a.temporary_area(&doc.id).unwrap();
    let outputs = store_a.outputs(&doc.id, Redactor::empty()).unwrap();
    assert!(!sessions.exists());
    // Using them creates only their own folders, which no listing shows.
    area.ensure().unwrap();
    drop(outputs.create().unwrap());
    assert_eq!(names(&folder), ["outputs", "tmp"]);
    assert!(store_a.list().unwrap().sessions.is_empty());
    assert!(store_a.list().unwrap().warnings.is_empty());

    // The first save adds the journal, its lock and the summary cache.
    store_a.acquire(&doc.id).unwrap().save(&doc).unwrap();
    assert_eq!(
        names(&folder),
        [
            "outputs",
            "session.jsonl",
            "session.lock",
            "summary.json",
            "tmp"
        ]
    );
    assert_eq!(names(store_a.bucket()), [doc.id.as_str()]);
    assert_eq!(names(&sessions).len(), 1);
    let listing = store_a.list().unwrap();
    assert_eq!(listing.sessions.len(), 1);
    assert_eq!(listing.sessions[0].id, doc.id);
    assert!(store_b.list().unwrap().sessions.is_empty());
    assert!(store_b.load(&doc.id).is_err());
    assert!(!store_b.session_directory(&doc.id).exists());
}

#[test]
fn unsupported_flat_session_is_reported_and_left_unchanged() {
    let home = Directory::new();
    let directory = Directory::new();
    let store = store(&home, &directory);
    let doc = document(&directory);
    // The earlier flat layout: journals beside each other in the bucket, with
    // bucket-level outputs and collaborators and a home-level temporary area.
    let legacy = Document::new(doc.directory.clone(), "fixture/model".into(), doc.effort).unwrap();
    let hash = store.bucket.file_name().unwrap().to_owned();
    let files = [
        store.bucket.join(format!("{}.json", doc.id)),
        store.bucket.join(format!("{}.jsonl", legacy.id)),
        store.bucket.join(format!("{}.lock", legacy.id)),
        store.bucket.join(format!("{}.summary.json", legacy.id)),
        store
            .bucket
            .join(format!("{}.jsonl.damaged-1-2-3", legacy.id)),
        store
            .bucket
            .join("outputs")
            .join(&legacy.id)
            .join("1-2-3.stdout"),
        store
            .bucket
            .join("collaboration")
            .join(&legacy.id)
            .join("parent-1.json"),
        home.path()
            .join("tmp")
            .join(&hash)
            .join(&legacy.id)
            .join("probe.txt"),
    ];
    for (index, path) in files.iter().enumerate() {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, format!("previous session data {index}")).unwrap();
    }
    for id in [&doc.id, &legacy.id] {
        for error in [
            store.load(id).err().unwrap(),
            store.create(id).err().unwrap(),
            Handle::open(store.clone(), id, Redactor::empty())
                .err()
                .unwrap(),
            store.delete(id).unwrap_err(),
        ] {
            assert!(error.contains("earlier Jecode version"), "{error}");
        }
        // No folder of the current layout appears beside the old files.
        assert!(!store.session_directory(id).exists());
    }
    let listing = store.list().unwrap();
    assert!(listing.sessions.is_empty());
    for id in [&doc.id, &legacy.id] {
        assert!(
            listing
                .warnings
                .iter()
                .any(|warning| warning == &format!("Session {id}: {}", crate::sessions::LEGACY)),
            "{:?}",
            listing.warnings
        );
    }
    for (index, path) in files.iter().enumerate() {
        assert_eq!(
            fs::read_to_string(path).unwrap(),
            format!("previous session data {index}")
        );
    }
}

#[cfg(unix)]
#[test]
fn an_inaccessible_session_is_reported_without_hiding_other_conversations() {
    use std::os::unix::fs::PermissionsExt;

    let home = Directory::new();
    let directory = Directory::new();
    let store = store(&home, &directory);
    let inaccessible = document(&directory);
    let other = document(&directory);
    for doc in [&inaccessible, &other] {
        store.acquire(&doc.id).unwrap().save(doc).unwrap();
    }
    let folder = store.session_directory(&inaccessible.id);
    let journal = store.journal_path(&inaccessible.id);
    let original = fs::read(&journal).unwrap();
    let permissions = fs::metadata(&folder).unwrap().permissions();
    fs::set_permissions(&folder, fs::Permissions::from_mode(0o000)).unwrap();
    let denied = fs::symlink_metadata(&journal).is_err();
    let listing = store.list();
    fs::set_permissions(&folder, permissions).unwrap();
    if !denied {
        // A privileged test runner may bypass the fixture's permissions.
        return;
    }

    let listing = listing.unwrap();
    assert_eq!(listing.sessions.len(), 1);
    assert_eq!(listing.sessions[0].id, other.id);
    assert!(
        listing
            .warnings
            .iter()
            .any(|warning| warning.starts_with(&format!("Session {}:", inaccessible.id))),
        "An inaccessible saved conversation must have a diagnostic: {:?}",
        listing.warnings
    );
    assert_eq!(fs::read(&journal).unwrap(), original);
}
