use super::*;
use crate::test_support::{Directory, link_directory};

const PARENT: &str = "1-2-3";
const CHILD: &str = "4-5-6";

/// `sessions/1-2-3/collaboration`, where the parent keeps its collaborators.
fn collaboration(sessions: &Path) -> std::path::PathBuf {
    sessions.join(PARENT).join("collaboration")
}

/// The child's own session folder inside the parent's collaborator home.
fn child_session(sessions: &Path) -> std::path::PathBuf {
    collaboration(sessions)
        .join("child-home")
        .join("sessions")
        .join("0123456789abcdef0123456789abcdef")
        .join(CHILD)
}

fn released(pool: &Pool, sessions: &Path, ids: &[&str]) -> Result<usize, String> {
    pool.collect(
        sessions,
        &BTreeSet::new(),
        &ids.iter().map(|id| id.to_string()).collect(),
    )
}

#[test]
fn released_parent_asset_survives_in_retained_child_journal() {
    let root = Directory::new();
    let sessions = root.path().join("sessions");
    let child = child_session(&sessions);
    fs::create_dir_all(&child).unwrap();
    let pool = Pool::new(sessions.join("attachments"));
    let kept = pool.import_bytes("kept.txt", b"child reference").unwrap();
    let orphan = pool.import_bytes("orphan.txt", b"unused").unwrap();

    // The parent journal has been deleted, while closing the collaborator
    // retained its journal beneath child-home.
    fs::write(child.join("session.jsonl"), kept.reference()).unwrap();
    fs::write(child.join("session.lock"), "").unwrap();
    fs::write(
        collaboration(&sessions).join("parent-1.json"),
        "retained collaboration",
    )
    .unwrap();
    assert_eq!(released(&pool, &sessions, &[&kept.id, &orphan.id]), Ok(1));
    assert!(pool.load(&kept.id).is_ok());
    assert!(pool.load(&orphan.id).is_err());
}

#[test]
fn retained_manifest_can_keep_a_reference_without_parent_journal() {
    let root = Directory::new();
    let sessions = root.path().join("sessions");
    let parent = collaboration(&sessions);
    fs::create_dir_all(&parent).unwrap();
    // Only the inert lock is left of the deleted parent's own records.
    fs::write(sessions.join(PARENT).join("session.lock"), "").unwrap();
    let pool = Pool::new(sessions.join("attachments"));
    let kept = pool
        .import_bytes("kept.txt", b"manifest reference")
        .unwrap();
    fs::write(parent.join("parent-1.json"), kept.reference()).unwrap();

    assert_eq!(released(&pool, &sessions, &[&kept.id]), Ok(0));
    assert!(pool.load(&kept.id).is_ok());
}

#[test]
fn neighbors_and_recovery_files_keep_their_assets() {
    let root = Directory::new();
    let sessions = root.path().join("sessions");
    let pool = Pool::new(sessions.join("attachments"));
    let journal = pool.import_bytes("journal.txt", b"a").unwrap();
    let damaged = pool.import_bytes("damaged.txt", b"b").unwrap();
    let summary = pool.import_bytes("summary.txt", b"c").unwrap();
    let orphan = pool.import_bytes("orphan.txt", b"d").unwrap();
    let neighbor = sessions.join("7-8-9");
    fs::create_dir_all(&neighbor).unwrap();
    fs::write(neighbor.join("session.jsonl"), journal.reference()).unwrap();
    fs::write(
        neighbor.join("session.jsonl.damaged-1-1-1"),
        damaged.reference(),
    )
    .unwrap();
    fs::write(neighbor.join("summary.tmp-1-1-1"), summary.reference()).unwrap();

    assert_eq!(
        released(
            &pool,
            &sessions,
            &[&journal.id, &damaged.id, &summary.id, &orphan.id]
        ),
        Ok(1)
    );
    for kept in [&journal, &damaged, &summary] {
        assert!(pool.load(&kept.id).is_ok());
    }
    assert!(pool.load(&orphan.id).is_err());
}

#[test]
fn outputs_temporary_files_and_workspaces_are_not_records() {
    let root = Directory::new();
    let sessions = root.path().join("sessions");
    let pool = Pool::new(sessions.join("attachments"));
    let orphan = pool.import_bytes("orphan.txt", b"unused").unwrap();
    let parent = sessions.join(PARENT);
    for folder in [
        parent.join("outputs"),
        parent.join("tmp"),
        collaboration(&sessions).join("a1").join("tree"),
        child_session(&sessions).join("tmp"),
        child_session(&sessions).join("outputs"),
    ] {
        fs::create_dir_all(&folder).unwrap();
        fs::write(folder.join("notes.jsonl"), orphan.reference()).unwrap();
    }
    fs::write(parent.join("session.jsonl"), "{}").unwrap();

    assert_eq!(released(&pool, &sessions, &[&orphan.id]), Ok(1));
    assert!(pool.load(&orphan.id).is_err());
}

#[test]
fn unsupported_flat_layout_histories_keep_their_assets() {
    let root = Directory::new();
    let sessions = root.path().join("sessions");
    let legacy_child = sessions
        .join("collaboration")
        .join(PARENT)
        .join("child-home")
        .join("sessions")
        .join("0123456789abcdef0123456789abcdef");
    fs::create_dir_all(&legacy_child).unwrap();
    let pool = Pool::new(sessions.join("attachments"));
    let flat = pool.import_bytes("flat.txt", b"a").unwrap();
    let child = pool.import_bytes("child.txt", b"b").unwrap();
    let manifest = pool.import_bytes("manifest.txt", b"c").unwrap();
    let orphan = pool.import_bytes("orphan.txt", b"d").unwrap();
    fs::write(sessions.join("7-8-9.jsonl"), flat.reference()).unwrap();
    fs::write(sessions.join("7-8-9.lock"), "").unwrap();
    fs::write(
        legacy_child.join(format!("{CHILD}.jsonl")),
        child.reference(),
    )
    .unwrap();
    fs::write(
        sessions
            .join("collaboration")
            .join(PARENT)
            .join("parent-1.json"),
        manifest.reference(),
    )
    .unwrap();

    assert_eq!(
        released(
            &pool,
            &sessions,
            &[&flat.id, &child.id, &manifest.id, &orphan.id]
        ),
        Ok(1)
    );
    for kept in [&flat, &child, &manifest] {
        assert!(pool.load(&kept.id).is_ok());
    }
    assert!(pool.load(&orphan.id).is_err());
}

#[test]
fn unreadable_child_journal_stops_collection_before_removal() {
    let root = Directory::new();
    let sessions = root.path().join("sessions");
    let child = child_session(&sessions);
    fs::create_dir_all(child.join("session.jsonl")).unwrap();
    let pool = Pool::new(sessions.join("attachments"));
    let asset = pool.import_bytes("asset.txt", b"owned").unwrap();

    assert!(
        released(&pool, &sessions, &[&asset.id])
            .unwrap_err()
            .contains("not a file")
    );
    assert!(pool.load(&asset.id).is_ok());
}

#[cfg(unix)]
#[test]
fn a_special_child_journal_stops_collection_before_removal() {
    let root = Directory::new();
    let sessions = root.path().join("sessions");
    let child = child_session(&sessions);
    fs::create_dir_all(&child).unwrap();
    let journal = child.join("session.jsonl");
    let status = std::process::Command::new(crate::native::helper("process_unix").unwrap())
        .arg("fifo")
        .arg(&journal)
        .status()
        .unwrap();
    assert!(status.success());
    let pool = Pool::new(sessions.join("attachments"));
    let asset = pool.import_bytes("asset.txt", b"owned").unwrap();

    assert!(
        released(&pool, &sessions, &[&asset.id])
            .unwrap_err()
            .contains("not a regular file")
    );
    assert!(pool.load(&asset.id).is_ok());
}

#[test]
fn linked_child_home_or_session_folder_is_not_followed_or_collected() {
    let outside = Directory::new();
    for linked in [
        |sessions: &Path| collaboration(sessions).join("child-home"),
        |sessions: &Path| sessions.join("7-8-9"),
        |sessions: &Path| child_session(sessions),
    ] {
        let root = Directory::new();
        let sessions = root.path().join("sessions");
        let link = linked(&sessions);
        fs::create_dir_all(link.parent().unwrap()).unwrap();
        link_directory(outside.path(), &link);
        let pool = Pool::new(sessions.join("attachments"));
        let asset = pool.import_bytes("asset.txt", b"owned").unwrap();

        assert!(released(&pool, &sessions, &[&asset.id]).is_err());
        assert!(pool.load(&asset.id).is_ok());
    }
}
