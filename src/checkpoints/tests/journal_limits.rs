use super::*;
use crate::checkpoints::frame::{self, HEADER};
use crate::checkpoints::quota;
use crate::checkpoints::record::{Event, Outcome, Point, Record};

#[test]
fn oversized_milestones_are_rejected_before_any_frame_is_appended() {
    let fixture = Fixture::new();
    let workspace = fixture.open();
    let before = workspace.store().journal.stored_bytes();
    let label = "x".repeat(frame::METADATA_LIMIT as usize);
    let result = workspace.mark("milestone", &label, "test", true, false);
    let after = workspace.store().journal.stored_bytes();
    drop(workspace);
    let reopened = fixture.open();
    let retained = result
        .as_ref()
        .is_ok_and(|seq| index(&reopened).points.contains_key(seq));
    assert!(
        result.is_err(),
        "oversized milestone returned {result:?}; retained after reopening: {retained}"
    );
    assert_eq!(before, after, "a rejected frame must not reach the journal");
    assert!(reopened.store().journal.damage.is_empty());
}

#[test]
fn oversized_pins_leave_the_original_point_and_journal_unchanged() {
    let fixture = Fixture::new();
    let workspace = fixture.open();
    let target = workspace
        .mark("milestone", "keep", "test", false, false)
        .unwrap();
    let before = workspace.store().journal.stored_bytes();
    let result = workspace.pin(target, &"x".repeat(frame::METADATA_LIMIT as usize), true);
    assert!(result.is_err(), "an oversized pin was accepted");
    assert_eq!(before, workspace.store().journal.stored_bytes());
    assert!(!index(&workspace).pins.contains_key(&target));
    drop(workspace);
    let reopened = fixture.open();
    assert!(index(&reopened).points.contains_key(&target));
    assert!(!index(&reopened).pins.contains_key(&target));
    assert!(reopened.store().journal.damage.is_empty());
}

#[test]
fn a_metadata_frame_at_the_replay_limit_survives_reopening() {
    let fixture = Fixture::new();
    let workspace = fixture.open();
    let point = workspace
        .store()
        .transaction(true, |transaction| {
            let mut point = Point {
                seq: transaction.seq(),
                kind: "milestone".into(),
                label: String::new(),
                origin: "test".into(),
                time: 0,
                policy: transaction.store.policy.id(),
                pinned: true,
            };
            let overhead = Record::Point(point.clone()).frame().len() - HEADER;
            point.label = "x".repeat(frame::METADATA_LIMIT as usize - overhead);
            assert_eq!(
                Record::Point(point.clone()).frame().len() - HEADER,
                frame::METADATA_LIMIT as usize
            );
            transaction.record(Record::Point(point.clone()));
            Ok(point)
        })
        .unwrap();
    drop(workspace);
    let reopened = fixture.open();
    let snapshot = index(&reopened);
    assert_eq!(snapshot.points.get(&point.seq), Some(&point));
    assert!(snapshot.pins.contains_key(&point.seq));
    assert!(reopened.store().journal.damage.is_empty());
}

#[test]
fn nonterminal_metadata_cannot_exhaust_a_prepared_writes_completion_reserve() {
    let fixture = Fixture::new();
    fixture.write("a.txt", b"before\n");
    let workspace = fixture.open();
    let mut mutation = workspace.mutation("session-a", "write", &fixture.path("a.txt"));
    mutation.prepare(b"after\n");
    assert_eq!(index(&workspace).pending.len(), 1);
    let filling = {
        let mut store = workspace.store();
        let used = quota::measure(&store.workspace.directory, store.journal.generation).unwrap();
        let limit = 1 << 20;
        store.journal.quota_limit = limit;
        let completion_bytes = Record::Complete {
            seq: 0,
            operation: 0,
            outcome: Outcome::Unknown,
        }
        .frame()
        .len() as u64;
        store.transaction(true, |transaction| {
            let mut event = Event {
                seq: transaction.seq(),
                origin: "other-session".into(),
                time: 0,
                text: String::new(),
                paths: Vec::new(),
            };
            let overhead = Record::Event(event.clone()).frame().len() as u64;
            let bytes = limit / 2 - used.generation - completion_bytes + 1;
            event.text = "x".repeat((bytes - overhead) as usize);
            transaction.record(Record::Event(event));
            Ok(())
        })
    };
    fixture.write("a.txt", b"after\n");
    let _protection = mutation.finish(true);
    assert!(
        index(&workspace).pending.is_empty(),
        "ordinary metadata consumed the space needed to complete a prepared write"
    );
    assert!(quota::is_full(&filling.unwrap_err()));
    workspace.store().journal.quota_limit = quota::LIMIT;
    assert_eq!(fixture.read("a.txt").unwrap(), b"after\n");
}
