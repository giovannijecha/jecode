use super::*;
use crate::test_support::{Directory, link_directory};

fn child_bucket(sessions: &Path) -> std::path::PathBuf {
    sessions
        .join("collaboration")
        .join("1-2-3")
        .join("child-home")
        .join("sessions")
        .join("0123456789abcdef0123456789abcdef")
}

#[test]
fn released_parent_asset_survives_in_retained_child_journal() {
    let root = Directory::new();
    let sessions = root.path().join("sessions");
    let child = child_bucket(&sessions);
    fs::create_dir_all(&child).unwrap();
    let pool = Pool::new(sessions.join("attachments"));
    let kept = pool.import_bytes("kept.txt", b"child reference").unwrap();
    let orphan = pool.import_bytes("orphan.txt", b"unused").unwrap();

    // The parent journal has been deleted, while closing the collaborator
    // retained its journal beneath child-home.
    fs::write(child.join("4-5-6.jsonl"), kept.reference()).unwrap();
    fs::write(
        sessions
            .join("collaboration")
            .join("1-2-3")
            .join("parent-1.json"),
        "retained collaboration",
    )
    .unwrap();
    let removed = pool
        .collect(
            &sessions,
            &BTreeSet::new(),
            &BTreeSet::from([kept.id.clone(), orphan.id.clone()]),
        )
        .unwrap();
    assert_eq!(removed, 1);
    assert!(pool.load(&kept.id).is_ok());
    assert!(pool.load(&orphan.id).is_err());
}

#[test]
fn retained_manifest_can_keep_a_reference_without_parent_journal() {
    let root = Directory::new();
    let sessions = root.path().join("sessions");
    let child = child_bucket(&sessions);
    let parent = child.parent().unwrap().parent().unwrap().parent().unwrap();
    fs::create_dir_all(parent).unwrap();
    let pool = Pool::new(sessions.join("attachments"));
    let kept = pool
        .import_bytes("kept.txt", b"manifest reference")
        .unwrap();
    fs::write(parent.join("parent-1.json"), kept.reference()).unwrap();

    assert_eq!(
        pool.collect(
            &sessions,
            &BTreeSet::new(),
            &BTreeSet::from([kept.id.clone()]),
        )
        .unwrap(),
        0
    );
    assert!(pool.load(&kept.id).is_ok());
}

#[test]
fn collection_does_not_treat_workspace_files_as_child_journals() {
    let root = Directory::new();
    let sessions = root.path().join("sessions");
    let workspace = sessions
        .join("collaboration")
        .join("1-2-3")
        .join("a1")
        .join("tree");
    fs::create_dir_all(&workspace).unwrap();
    let pool = Pool::new(sessions.join("attachments"));
    let orphan = pool.import_bytes("orphan.txt", b"unused").unwrap();
    fs::write(workspace.join("notes.jsonl"), orphan.reference()).unwrap();

    assert_eq!(
        pool.collect(
            &sessions,
            &BTreeSet::new(),
            &BTreeSet::from([orphan.id.clone()]),
        )
        .unwrap(),
        1
    );
    assert!(pool.load(&orphan.id).is_err());
}

#[test]
fn unreadable_child_journal_stops_collection_before_removal() {
    let root = Directory::new();
    let sessions = root.path().join("sessions");
    let child = child_bucket(&sessions);
    fs::create_dir_all(child.join("4-5-6.jsonl")).unwrap();
    let pool = Pool::new(sessions.join("attachments"));
    let asset = pool.import_bytes("asset.txt", b"owned").unwrap();

    assert!(
        pool.collect(
            &sessions,
            &BTreeSet::new(),
            &BTreeSet::from([asset.id.clone()]),
        )
        .unwrap_err()
        .contains("not a file")
    );
    assert!(pool.load(&asset.id).is_ok());
}

#[test]
fn linked_child_home_is_not_followed_or_collected() {
    let root = Directory::new();
    let sessions = root.path().join("sessions");
    let parent = sessions.join("collaboration").join("1-2-3");
    fs::create_dir_all(&parent).unwrap();
    let outside = Directory::new();
    link_directory(outside.path(), &parent.join("child-home"));
    let pool = Pool::new(sessions.join("attachments"));
    let asset = pool.import_bytes("asset.txt", b"owned").unwrap();

    assert!(
        pool.collect(
            &sessions,
            &BTreeSet::new(),
            &BTreeSet::from([asset.id.clone()]),
        )
        .is_err()
    );
    assert!(pool.load(&asset.id).is_ok());
}
