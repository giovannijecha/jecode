use super::*;
use crate::checkpoints::hash;
use crate::checkpoints::record::{Discovery, Recovery, RecoveryAction, Version};
use crate::checkpoints::recovery;
use std::sync::Arc;

#[test]
fn sweep_compacts_expired_recovery_in_a_closed_archive_without_workspace_access() {
    let (fixture, active) = fixture();
    let other_root = fixture.path().join("other-project");
    fs::create_dir(&other_root).unwrap();
    let home = fixture.path().join("home");
    let other = identity::locate(&home, &other_root, true).unwrap().unwrap();
    let active_store = Store::open(active).unwrap();
    let mut closed_store = Store::open(other.clone()).unwrap();
    let (before_object, after_object) = closed_store
        .transaction(true, |tx| {
            let (group, snapshot) = recovery::begin_snapshot(tx, "owner", "write")?;
            tx.flush(false)?;
            let before: Arc<[u8]> = b"before".as_slice().into();
            let after: Arc<[u8]> = b"after".as_slice().into();
            let before_object = tx.object(before.clone(), None)?;
            recovery::add(
                tx,
                group,
                snapshot,
                "b.txt",
                &State::File {
                    object: before_object,
                    length: before.len() as u64,
                    digest: hash::digest(&before),
                    mode: Default::default(),
                },
            )?;
            let after_object = tx.object(after.clone(), None)?;
            let version_seq = tx.seq();
            tx.record(crate::checkpoints::record::Record::Version(Version {
                seq: version_seq,
                path: "b.txt".into(),
                state: State::File {
                    object: after_object,
                    length: after.len() as u64,
                    digest: hash::digest(&after),
                    mode: Default::default(),
                },
                stat: None,
                time: crate::checkpoints::clock::now(),
                origin: "owner".into(),
                operation: 0,
                discovery: Discovery::Known,
            }));
            let now = crate::checkpoints::clock::now();
            let accepted = now - 2 * recovery::MAX_ACCEPTED_MS;
            let status_seq = tx.seq();
            tx.record(crate::checkpoints::record::Record::Recovery(
                Recovery::Status {
                    seq: status_seq,
                    group,
                    action: RecoveryAction::Accepted,
                    time: accepted,
                    until: accepted + recovery::MAX_ACCEPTED_MS,
                },
            ));
            Ok((before_object, after_object))
        })
        .unwrap();
    fs::write(other_root.join("b.txt"), b"after").unwrap();
    touch(&other.directory).unwrap();
    let last_use = fs::read(last_use_path(&other.directory)).unwrap();
    let old_generation = closed_store.journal.generation;
    drop(closed_store);
    assert!(super::super::idle_gc::due(&other.directory));

    assert_eq!(sweep(&home.join("checkpoints")).unwrap(), 0);
    assert_eq!(fs::read(last_use_path(&other.directory)).unwrap(), last_use);
    assert_eq!(fs::read(other_root.join("b.txt")).unwrap(), b"after");
    let reopened = Store::open(other).unwrap();
    assert!(reopened.journal.generation > old_generation);
    assert!(!reopened.index.objects.contains_key(&before_object));
    assert!(!reopened.index.objects.contains_key(&after_object));
    assert!(matches!(
        reopened.index.current_state("b.txt"),
        State::UnretainedFile { .. }
    ));
    drop(active_store);
}

#[test]
fn recent_active_group_blocks_expiry_even_with_old_archive_activity() {
    let (_fixture, location) = fixture();
    let now = crate::checkpoints::clock::now();
    let mut store = Store::open(location.clone()).unwrap();
    let group = store
        .transaction(true, |tx| {
            let mode_seq = tx.seq();
            tx.record(crate::checkpoints::record::Record::Recovery(
                Recovery::Mode {
                    seq: mode_seq,
                    time: now - 8 * recovery::MAX_ACCEPTED_MS,
                },
            ));
            let (group, _) = recovery::begin_snapshot(tx, "owner", "write")?;
            Ok(group)
        })
        .unwrap();
    drop(store);
    age(&location.directory);
    let activity = fs::read(last_use_path(&location.directory)).unwrap();

    assert_eq!(sweep(location.directory.parent().unwrap()).unwrap(), 0);
    assert!(layout::history(&location.directory).exists());
    assert_eq!(
        fs::read(last_use_path(&location.directory)).unwrap(),
        activity
    );
    let reopened = Store::open(location).unwrap();
    assert!(recovery::available(&reopened.index, group));
}
