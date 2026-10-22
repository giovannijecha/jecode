use super::*;
use crate::checkpoints::frame::{HEADER, Kind};
use crate::checkpoints::record::{Discovery, Recovery, Stat, Version};

fn point(seq: u64, kind: &str, time: u64) -> Point {
    Point {
        seq,
        kind: kind.into(),
        label: String::new(),
        origin: "owner".into(),
        time,
        policy: 0,
        pinned: false,
    }
}

fn group_with_steps() -> Index {
    let mut index = Index::default();
    let now = crate::checkpoints::clock::now();
    for record in [
        Record::Recovery(Recovery::Mode { seq: 1, time: now }),
        Record::Point(point(2, GROUP_KIND, now)),
        Record::Point(point(3, SNAPSHOT_KIND, now)),
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
        Record::Recovery(Recovery::Scope {
            seq: 6,
            group: 2,
            snapshot: 3,
            root: "src".into(),
        }),
        Record::Point(point(7, SNAPSHOT_KIND, now + 20)),
        Record::Recovery(Recovery::Snapshot {
            seq: 8,
            group: 2,
            snapshot: 7,
        }),
        Record::Recovery(Recovery::Step {
            seq: 9,
            group: 2,
            snapshot: 7,
            path: "src/a".into(),
            before: State::Directory {
                mode: Default::default(),
            },
        }),
    ] {
        index.record(record);
    }
    index
}

#[test]
fn accepted_group_uses_earliest_path_state_and_scope_absence() {
    let mut index = group_with_steps();
    let now = crate::checkpoints::clock::now();
    assert_eq!(target(&index, 2, "src/a"), Some(State::Absent));
    assert_eq!(target(&index, 2, "src/new"), Some(State::Absent));
    assert_eq!(target(&index, 2, "src/target/ignored.bin"), None);
    assert!(available_at(
        &index,
        2,
        now + 100 * DAY,
        Expiry {
            accepted_ms: DAY,
            active_ms: None,
        }
    ));
    index.record(Record::Recovery(Recovery::Status {
        seq: 10,
        group: 2,
        action: RecoveryAction::Accepted,
        time: now + 30,
        until: now + 30 + DAY,
    }));
    assert!(!available_at(&index, 7, now + 31, Expiry::default()));
    assert!(available_at(
        &index,
        2,
        now + 30 + DAY - 1,
        Expiry::default()
    ));
    assert!(available_at(
        &index,
        2,
        now + 30 + DAY - 1,
        Expiry {
            accepted_ms: 0,
            active_ms: Some(DAY),
        }
    ));
    assert!(!available_at(&index, 2, now + 30 + DAY, Expiry::default()));
    index.record(Record::Pin {
        seq: 11,
        target: 2,
        pinned: true,
        label: "keep".into(),
    });
    assert!(available_at(
        &index,
        2,
        now + 30 + 20 * DAY,
        Expiry::default()
    ));
    index.record(Record::Pin {
        seq: 12,
        target: 2,
        pinned: false,
        label: String::new(),
    });
    assert!(!available_at(
        &index,
        2,
        now + 30 + DAY,
        Expiry {
            accepted_ms: 30 * DAY,
            active_ms: None,
        }
    ));
    assert_eq!(target(&index, 2, "src/a"), Some(State::Absent));
}

#[test]
fn recovery_frame_round_trips_before_state() {
    for record in [
        Record::Recovery(Recovery::Step {
            seq: 12,
            group: 2,
            snapshot: 3,
            path: "src/main.rs".into(),
            before: State::Absent,
        }),
        Record::Recovery(Recovery::Status {
            seq: 13,
            group: 2,
            action: RecoveryAction::Accepted,
            time: 100,
            until: 100 + DAY,
        }),
    ] {
        let frame = record.frame();
        assert_eq!(
            Record::decode(Kind::Recovery, &frame[HEADER..]).unwrap(),
            record
        );
    }
}

#[test]
fn unretained_head_is_not_recoverable_but_compares_by_content() {
    let digest = crate::checkpoints::hash::digest(b"content");
    let file = State::File {
        object: 42,
        length: 7,
        digest,
        mode: Default::default(),
    };
    let head = State::UnretainedFile {
        length: 7,
        digest,
        mode: Default::default(),
    };
    assert!(file.same(&head));
    assert!(!head.recoverable());
    let mut index = Index::default();
    index.record(Record::Version(Version {
        seq: 1,
        path: "src/a".into(),
        state: head,
        stat: Some(Stat::default()),
        time: 1,
        origin: "owner".into(),
        operation: 0,
        discovery: Discovery::Known,
    }));
    assert_eq!(index.current("src/a").unwrap().seq, 1);
}

#[test]
fn touch_renews_only_active_inactivity() {
    let mut index = group_with_steps();
    let started = index.recovery.groups.get(&2).unwrap().created;
    let touched = started + 6 * DAY;
    index.record(Record::Recovery(Recovery::Status {
        seq: 10,
        group: 2,
        action: RecoveryAction::Touched,
        time: touched,
        until: 0,
    }));
    assert!(available_at(
        &index,
        2,
        touched + 7 * DAY - 1,
        Expiry::default()
    ));
    assert!(!available_at(
        &index,
        2,
        touched + 7 * DAY,
        Expiry::default()
    ));
}

#[test]
fn generic_points_cannot_extend_targeted_retention() {
    let mut index = Index::default();
    let now = crate::checkpoints::clock::now();
    index.record(Record::Point(point(1, "milestone", now - 9 * DAY)));
    index.record(Record::Recovery(Recovery::Mode {
        seq: 2,
        time: now - 8 * DAY,
    }));
    index.record(Record::Point(point(3, "milestone", now - 2 * DAY)));
    index.record(Record::Point(point(4, "milestone", now - DAY / 2)));
    assert!(!index.retained(1));
    assert!(!index.retained(3));
    assert!(index.retained(4));
    index.record(Record::Pin {
        seq: 5,
        target: 3,
        pinned: true,
        label: "keep".into(),
    });
    assert!(index.retained(3));
}

#[test]
fn a_generic_pin_cannot_restore_a_metadata_only_head() {
    let mut index = Index::default();
    let now = crate::checkpoints::clock::now();
    index.record(Record::Recovery(Recovery::Mode { seq: 1, time: now }));
    index.record(Record::Version(Version {
        seq: 2,
        path: "a.txt".into(),
        state: State::UnretainedFile {
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
    index.record(Record::Point(point(3, "milestone", now)));
    index.record(Record::Pin {
        seq: 4,
        target: 3,
        pinned: true,
        label: "keep".into(),
    });
    assert!(!index.retained(3));
}

#[test]
fn legacy_nonpoint_references_keep_their_grace_period() {
    let mut index = Index::default();
    let now = crate::checkpoints::clock::now();
    index.record(Record::Version(Version {
        seq: 1,
        path: "a.txt".into(),
        state: State::Absent,
        stat: None,
        time: now,
        origin: "owner".into(),
        operation: 0,
        discovery: Discovery::Known,
    }));
    index.record(Record::Recovery(Recovery::Mode { seq: 2, time: now }));
    assert!(index.retained(1));
    index.horizon = 2;
    assert!(!index.retained(1));
    index.record(Record::Pin {
        seq: 3,
        target: 1,
        pinned: true,
        label: "keep".into(),
    });
    assert!(index.retained(1));
}

#[test]
fn an_uncaptured_large_path_does_not_hide_other_paths_at_a_milestone() {
    let mut index = Index::default();
    let now = crate::checkpoints::clock::now();
    index.record(Record::Recovery(Recovery::Mode { seq: 1, time: now }));
    index.record(Record::Version(Version {
        seq: 2,
        path: "large.bin".into(),
        state: State::Oversized { length: 10_000 },
        stat: None,
        time: now,
        origin: "owner".into(),
        operation: 0,
        discovery: Discovery::Known,
    }));
    index.record(Record::Point(point(3, "milestone", now)));
    assert!(index.retained(3));
}
