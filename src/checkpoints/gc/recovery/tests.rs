use super::*;
use crate::checkpoints::identity;
use crate::checkpoints::record::{Discovery, Outcome, RecoveryAction};
use crate::checkpoints::tests::Fixture;
use std::sync::Arc;

fn point(seq: u64, kind: &str, time: u64) -> Record {
    Record::Point(Point {
        seq,
        kind: kind.into(),
        label: String::new(),
        origin: "owner".into(),
        time,
        policy: 0,
        pinned: false,
    })
}

#[test]
fn superseded_touch_does_not_move_a_live_generation() {
    let fixture = Fixture::new();
    let workspace = fixture.open();
    let location = identity::locate(fixture.home.path(), fixture.root.path(), false)
        .unwrap()
        .unwrap();
    let mut store = workspace.store();
    let group = store
        .transaction(true, |tx| {
            let (group, snapshot) = recovery::begin_snapshot(tx, "owner", "write")?;
            tx.flush(false)?;
            let bytes: Arc<[u8]> = b"held before-image".as_slice().into();
            let object = tx.object(bytes.clone(), None)?;
            recovery::add(
                tx,
                group,
                snapshot,
                "a.txt",
                &State::File {
                    object,
                    length: bytes.len() as u64,
                    digest: crate::checkpoints::hash::digest(&bytes),
                    mode: Default::default(),
                },
            )?;
            let now = crate::checkpoints::clock::now();
            for offset in [1, 2] {
                let seq = tx.seq();
                tx.record(Record::Recovery(Recovery::Status {
                    seq,
                    group,
                    action: RecoveryAction::Touched,
                    time: now + offset,
                    until: 0,
                }));
            }
            Ok(group)
        })
        .unwrap();
    let generation = store.journal.generation;
    assert!(recovery::available(&store.index, group));
    drop(store);
    assert!(super::super::collect(&location, false).unwrap().is_none());
    let mut store = workspace.store();
    store.refresh().unwrap();
    assert_eq!(store.journal.generation, generation);
    assert!(recovery::available(&store.index, group));
}

#[test]
fn duplicate_step_with_held_object_does_not_require_collection() {
    let mut index = Index::default();
    let now = crate::checkpoints::clock::now();
    let before = State::File {
        object: 50,
        length: 4,
        digest: crate::checkpoints::hash::digest(b"data"),
        mode: Default::default(),
    };
    for record in [
        Record::Recovery(Recovery::Mode { seq: 1, time: now }),
        point(2, GROUP_KIND, now),
        point(3, SNAPSHOT_KIND, now),
        Record::Recovery(Recovery::Snapshot {
            seq: 4,
            group: 2,
            snapshot: 3,
        }),
        Record::Recovery(Recovery::Step {
            seq: 5,
            group: 2,
            snapshot: 3,
            path: "a.txt".into(),
            before: before.clone(),
        }),
        Record::Recovery(Recovery::Step {
            seq: 6,
            group: 2,
            snapshot: 3,
            path: "a.txt".into(),
            before,
        }),
    ] {
        index.record(record);
    }
    assert!(!due(&index, now));
}

#[test]
fn acceptance_collapses_steps_and_current_head_loses_payload_reference() {
    let mut index = Index::default();
    let digest = crate::checkpoints::hash::digest(b"after");
    let file = State::File {
        object: 50,
        length: 5,
        digest,
        mode: Default::default(),
    };
    for record in [
        Record::Recovery(Recovery::Mode { seq: 1, time: 10 }),
        point(2, GROUP_KIND, 10),
        point(3, SNAPSHOT_KIND, 10),
        Record::Recovery(Recovery::Snapshot {
            seq: 4,
            group: 2,
            snapshot: 3,
        }),
        Record::Recovery(Recovery::Step {
            seq: 5,
            group: 2,
            snapshot: 3,
            path: "src/a".into(),
            before: State::Absent,
        }),
        point(6, SNAPSHOT_KIND, 20),
        Record::Recovery(Recovery::Snapshot {
            seq: 7,
            group: 2,
            snapshot: 6,
        }),
        Record::Recovery(Recovery::Step {
            seq: 8,
            group: 2,
            snapshot: 6,
            path: "src/a".into(),
            before: file.clone(),
        }),
        Record::Version(Version {
            seq: 9,
            path: "src/a".into(),
            state: file,
            stat: None,
            time: 20,
            origin: "owner".into(),
            operation: 0,
            discovery: Discovery::Known,
        }),
        Record::Recovery(Recovery::Status {
            seq: 10,
            group: 2,
            action: RecoveryAction::Accepted,
            time: 30,
            until: 30 + recovery::MAX_ACCEPTED_MS,
        }),
    ] {
        index.record(record);
    }
    assert!(due(&index, 31));
    let records = snapshot(&index, 1, 31);
    assert!(
        records
            .iter()
            .any(|record| matches!(record, Record::Recovery(Recovery::Step { seq: 5, .. })))
    );
    assert!(
        !records
            .iter()
            .any(|record| matches!(record, Record::Recovery(Recovery::Step { seq: 8, .. })))
    );
    assert!(records.iter().any(|record| matches!(
        record,
        Record::Version(Version {
            state: State::UnretainedFile { .. },
            ..
        })
    )));
}

#[test]
fn collection_keeps_before_image_and_drops_unheld_after_image() {
    let fixture = Fixture::new();
    let workspace = fixture.open();
    let location = identity::locate(fixture.home.path(), fixture.root.path(), false)
        .unwrap()
        .unwrap();
    let mut store = workspace.store();
    let (group, before_object, after_object) = store
        .transaction(true, |tx| {
            let (group, snapshot) = recovery::begin_snapshot(tx, "owner", "write")?;
            tx.flush(false)?;
            let before: Arc<[u8]> = b"before".as_slice().into();
            let after: Arc<[u8]> = b"after".as_slice().into();
            let before_object = tx.object(before.clone(), None)?;
            let before_state = State::File {
                object: before_object,
                length: before.len() as u64,
                digest: crate::checkpoints::hash::digest(&before),
                mode: Default::default(),
            };
            recovery::add(tx, group, snapshot, "a.txt", &before_state)?;
            let after_object = tx.object(after.clone(), None)?;
            let seq = tx.seq();
            tx.record(Record::Version(Version {
                seq,
                path: "a.txt".into(),
                state: State::File {
                    object: after_object,
                    length: after.len() as u64,
                    digest: crate::checkpoints::hash::digest(&after),
                    mode: Default::default(),
                },
                stat: None,
                time: crate::checkpoints::clock::now(),
                origin: "owner".into(),
                operation: 0,
                discovery: Discovery::Known,
            }));
            Ok((group, before_object, after_object))
        })
        .unwrap();
    store
        .transaction(true, |tx| recovery::accept(tx, group, "owner"))
        .unwrap();
    drop(store);
    super::super::collect(&location, true).unwrap().unwrap();
    let mut store = workspace.store();
    store.refresh().unwrap();
    assert!(store.index.objects.contains_key(&before_object));
    assert!(!store.index.objects.contains_key(&after_object));
    assert!(matches!(
        store.index.current_state("a.txt"),
        State::UnretainedFile { .. }
    ));
    assert!(matches!(
        recovery::target(&store.index, group, "a.txt"),
        Some(State::File { object, .. }) if object == before_object
    ));
}

#[test]
fn pinned_generic_point_keeps_its_post_mode_file() {
    let mut index = Index::default();
    let now = crate::checkpoints::clock::now();
    index.record(Record::Recovery(Recovery::Mode { seq: 1, time: now }));
    index.record(Record::Version(Version {
        seq: 2,
        path: "a.txt".into(),
        state: State::File {
            object: 40,
            length: 4,
            digest: crate::checkpoints::hash::digest(b"data"),
            mode: Default::default(),
        },
        stat: None,
        time: now,
        origin: "owner".into(),
        operation: 0,
        discovery: Discovery::Known,
    }));
    index.record(point(3, "milestone", now));
    index.record(Record::Pin {
        seq: 4,
        target: 3,
        pinned: true,
        label: "keep".into(),
    });
    assert!(snapshot(&index, 3, now).iter().any(|record| matches!(
        record,
        Record::Version(Version {
            seq: 2,
            state: State::File { object: 40, .. },
            ..
        })
    )));
    assert!(!due(&index, now));
    index.record(Record::Pin {
        seq: 5,
        target: 3,
        pinned: false,
        label: String::new(),
    });
    assert!(snapshot(&index, 3, now).iter().any(|record| matches!(
        record,
        Record::Version(Version {
            seq: 2,
            state: State::File { object: 40, .. },
            ..
        })
    )));
    let expired = now + recovery::MAX_ACCEPTED_MS;
    assert!(snapshot(&index, 3, expired).iter().any(|record| matches!(
        record,
        Record::Version(Version {
            seq: 2,
            state: State::UnretainedFile { .. },
            ..
        })
    )));
    assert!(due(&index, expired));
}

#[test]
fn legacy_latest_point_keeps_its_state_when_a_later_change_precedes_mode() {
    let mut index = Index::default();
    let now = crate::checkpoints::clock::now();
    for (seq, bytes) in [(1, b"old".as_slice()), (3, b"new".as_slice())] {
        index.record(Record::Version(Version {
            seq,
            path: "a.txt".into(),
            state: State::File {
                object: seq + 30,
                length: bytes.len() as u64,
                digest: crate::checkpoints::hash::digest(bytes),
                mode: Default::default(),
            },
            stat: None,
            time: now,
            origin: "owner".into(),
            operation: 0,
            discovery: Discovery::Known,
        }));
    }
    index.record(point(2, "milestone", now));
    index.record(Record::Recovery(Recovery::Mode { seq: 4, time: now }));
    let records = snapshot(&index, 4, now);
    assert!(records.iter().any(|record| matches!(
        record,
        Record::Version(Version {
            seq: 1,
            state: State::File { .. },
            ..
        })
    )));
    assert!(records.iter().any(|record| matches!(
        record,
        Record::Version(Version {
            seq: 3,
            state: State::File { .. },
            ..
        })
    )));
}

#[test]
fn recent_completion_outcomes_survive_targeted_collection() {
    let mut index = Index::default();
    index.record(Record::Recovery(Recovery::Mode { seq: 1, time: 10 }));
    index.record(Record::Complete {
        seq: 3,
        operation: 2,
        outcome: Outcome::Applied,
    });
    let records = snapshot(&index, 2, 11);
    assert!(records.iter().any(|record| matches!(
        record,
        Record::Complete {
            operation: 2,
            outcome: Outcome::Applied,
            ..
        }
    )));
}

#[test]
fn an_unpinned_milestone_releases_its_file_after_one_day() {
    let mut index = Index::default();
    let point_time = 30;
    index.record(Record::Recovery(Recovery::Mode { seq: 1, time: 10 }));
    index.record(Record::Version(Version {
        seq: 2,
        path: "a.txt".into(),
        state: State::File {
            object: 40,
            length: 4,
            digest: crate::checkpoints::hash::digest(b"data"),
            mode: Default::default(),
        },
        stat: None,
        time: 20,
        origin: "owner".into(),
        operation: 0,
        discovery: Discovery::Known,
    }));
    index.record(point(3, "milestone", point_time));
    let live = snapshot(&index, 3, point_time + recovery::MAX_ACCEPTED_MS - 1);
    assert!(live.iter().any(|record| matches!(
        record,
        Record::Version(Version {
            seq: 2,
            state: State::File { .. },
            ..
        })
    )));
    assert!(live.iter().any(|record| matches!(
        record,
        Record::Point(Point { seq: 3, kind, .. }) if kind == "milestone"
    )));
    let expired = point_time + recovery::MAX_ACCEPTED_MS;
    assert!(due(&index, expired));
    let released = snapshot(&index, 3, expired);
    assert!(released.iter().any(|record| matches!(
        record,
        Record::Version(Version {
            seq: 2,
            state: State::UnretainedFile { .. },
            ..
        })
    )));
    assert!(!released.iter().any(|record| matches!(
        record,
        Record::Point(Point { seq: 3, kind, .. }) if kind == "milestone"
    )));
}

#[test]
fn pinned_milestone_materializes_metadata_only_head_and_survives_collection() {
    let fixture = Fixture::new();
    fixture.write("a.txt", b"exact bytes");
    let workspace = fixture.open();
    let location = identity::locate(fixture.home.path(), fixture.root.path(), false)
        .unwrap()
        .unwrap();
    let mut store = workspace.store();
    store
        .transaction(true, |tx| {
            recovery::enable(tx)?;
            let bytes: Arc<[u8]> = b"exact bytes".as_slice().into();
            let object = tx.object(bytes.clone(), None)?;
            let seq = tx.seq();
            tx.record(Record::Version(Version {
                seq,
                path: "a.txt".into(),
                state: State::File {
                    object,
                    length: bytes.len() as u64,
                    digest: crate::checkpoints::hash::digest(&bytes),
                    mode: Default::default(),
                },
                stat: None,
                time: crate::checkpoints::clock::now(),
                origin: "owner".into(),
                operation: 0,
                discovery: Discovery::Known,
            }));
            Ok(())
        })
        .unwrap();
    drop(store);
    super::super::collect(&location, true).unwrap().unwrap();
    workspace.store().refresh().unwrap();
    assert!(matches!(
        workspace.store().index.current_state("a.txt"),
        State::UnretainedFile { .. }
    ));
    let milestone = workspace
        .mark("milestone", "kept", "owner", true, false)
        .unwrap();
    assert!(matches!(
        workspace.store().index.state_at("a.txt", milestone),
        State::File { .. }
    ));
    super::super::collect(&location, true).unwrap().unwrap();
    let mut store = workspace.store();
    store.refresh().unwrap();
    assert!(store.index.retained(milestone));
    let State::File { object, .. } = store.index.state_at("a.txt", milestone) else {
        panic!("pinned milestone lost its materialized file");
    };
    assert_eq!(&*store.bytes(object).unwrap(), b"exact bytes");
}
