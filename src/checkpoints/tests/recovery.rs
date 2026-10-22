use super::*;
use crate::checkpoints::files::Mode;
use crate::checkpoints::frame::{self, HEADER, Kind};
use crate::checkpoints::record::{Event, Outcome, Prepare, Record, State};
use crate::checkpoints::store::Store;
use std::io::Write;

/// The newest segment and its length after closing the workspace.
fn close(workspace: Arc<Workspace>) -> (PathBuf, u64) {
    let segment = {
        let store = workspace.store();
        store.journal.segment_path(store.journal.position().0)
    };
    drop(workspace);
    let length = fs::metadata(&segment).unwrap().len();
    (segment, length)
}

fn append(path: &PathBuf, bytes: &[u8]) {
    let mut file = fs::OpenOptions::new().append(true).open(path).unwrap();
    file.write_all(bytes).unwrap();
}

fn damage(workspace: &Workspace) -> String {
    workspace.store().journal.damage.join("\n")
}

#[test]
fn an_interrupted_append_is_removed_and_earlier_history_survives() {
    let frame = frame::encode(Kind::Event, &[7u8; 64]);
    let tails: [(&str, Vec<u8>); 2] = [
        ("a partial frame header", frame[..HEADER - 4].to_vec()),
        ("an unwritten frame", vec![0u8; 4096]),
    ];
    for (what, tail) in tails {
        let fixture = Fixture::new();
        fixture.write("a.txt", b"a\n");
        let workspace = fixture.open();
        let point = workspace
            .mark("milestone", "kept", "test", true, false)
            .unwrap();
        let (segment, length) = close(workspace);
        append(&segment, &tail);

        let reopened = fixture.open();
        let report = damage(&reopened);
        assert!(
            report.contains(what) && report.contains("interrupted write"),
            "{report}"
        );
        let index = index(&reopened);
        assert!(index.points.contains_key(&point) && index.pins.contains_key(&point));
        let (segment, grown) = close(reopened);
        assert!(grown > length);
        // Nothing is left to repair on the next open.
        let again = fixture.open();
        assert_eq!(damage(&again), "", "{}", segment.display());
    }
}

#[test]
fn a_suspect_frame_length_preserves_later_bytes_and_uses_a_new_segment() {
    for length in [4096u64, u64::MAX] {
        let fixture = Fixture::new();
        fixture.write("a.txt", b"a\n");
        let workspace = fixture.open();
        let before = workspace
            .mark("milestone", "before", "test", false, false)
            .unwrap();
        let (segment, original) = close(workspace);
        let later = Record::Event(Event {
            seq: before + 2,
            origin: "test".into(),
            time: 0,
            text: "valid frame after the damaged header".into(),
            paths: Vec::new(),
        })
        .frame();
        let mut damaged = frame::encode(Kind::Event, &[7u8; 64]);
        damaged[8..16].copy_from_slice(&length.to_le_bytes());
        append(&segment, &damaged);
        append(&segment, &later);
        let expected = original + damaged.len() as u64 + later.len() as u64;

        let reopened = fixture.open();
        assert!(damage(&reopened).contains("frame length exceeds the file"));
        assert_eq!(fs::metadata(&segment).unwrap().len(), expected);
        let after = reopened
            .mark("milestone", "after", "test", false, false)
            .unwrap();
        let (newest, _) = close(reopened);
        assert_ne!(newest, segment);
        assert_eq!(fs::metadata(&segment).unwrap().len(), expected);
        let index = index(&fixture.open());
        assert!(index.points.contains_key(&before) && index.points.contains_key(&after));
    }
}

#[test]
fn a_partial_payload_is_preserved_because_its_length_cannot_be_trusted() {
    let fixture = Fixture::new();
    fixture.write("a.txt", b"a\n");
    let workspace = fixture.open();
    let (segment, original) = close(workspace);
    let frame = frame::encode(Kind::Event, &[7u8; 64]);
    append(&segment, &frame[..frame.len() - 8]);
    let expected = original + (frame.len() - 8) as u64;

    let reopened = fixture.open();
    assert!(damage(&reopened).contains("frame length exceeds the file"));
    assert_eq!(fs::metadata(&segment).unwrap().len(), expected);
    reopened
        .mark("milestone", "after", "test", false, false)
        .unwrap();
    let (newest, _) = close(reopened);
    assert_ne!(newest, segment);
    assert_eq!(fs::metadata(&segment).unwrap().len(), expected);
}

#[test]
fn a_point_must_flush_the_journal_before_it_can_protect_a_command() {
    let fixture = Fixture::new();
    fixture.write("a.txt", b"a\n");
    let workspace = fixture.open();
    workspace.store().journal.sync_fault = Some("synthetic flush failure".into());
    let command = workspace.command("test", "change a.txt");
    assert_eq!(command.point, Err("synthetic flush failure".into()));
    drop(command);

    // Reusing a point needs the same barrier, including when its last frames
    // were appended by a different process.
    assert!(
        workspace
            .mark("milestone", "reused", "test", false, true)
            .is_err()
    );
    assert!(
        workspace
            .mark("milestone", "new", "test", false, false)
            .is_err()
    );
    workspace.store().journal.sync_fault = None;
    assert!(
        workspace
            .mark("milestone", "durable", "test", false, false)
            .is_ok()
    );
}

#[test]
fn damaged_complete_frames_are_kept_and_skipped() {
    let fixture = Fixture::new();
    fixture.write("a.txt", b"a\n");
    let workspace = fixture.open();
    let (segment, length) = close(workspace);
    let mut frame = frame::encode(Kind::Event, &[7u8; 64]);
    let last = frame.len() - 1;
    frame[last] ^= 0xff;
    append(&segment, &frame);

    let reopened = fixture.open();
    assert!(damage(&reopened).contains("checksum mismatch"));
    reopened
        .mark("milestone", "after damage", "test", false, false)
        .unwrap();
    drop(reopened);
    assert!(fs::metadata(&segment).unwrap().len() >= length + frame.len() as u64);
    assert!(
        index(&fixture.open())
            .points
            .values()
            .any(|point| point.label == "after damage")
    );
}

#[test]
fn unreadable_history_continues_in_a_new_segment() {
    let fixture = Fixture::new();
    fixture.write("a.txt", b"a\n");
    let workspace = fixture.open();
    let point = workspace
        .mark("milestone", "before", "test", false, false)
        .unwrap();
    let (segment, length) = close(workspace);
    // Bytes that are neither a frame nor an unwritten tail.
    append(&segment, &[0x5a; HEADER * 2]);

    let reopened = fixture.open();
    assert!(damage(&reopened).contains("unreadable"));
    let later = reopened
        .mark("milestone", "after", "test", false, false)
        .unwrap();
    let (newest, _) = close(reopened);
    assert_ne!(newest, segment);
    // The damaged bytes are preserved for inspection.
    assert_eq!(
        fs::metadata(&segment).unwrap().len(),
        length + HEADER as u64 * 2
    );
    let index = index(&fixture.open());
    assert!(index.points.contains_key(&point) && index.points.contains_key(&later));
}

/// Records an operation as a process that exited before completing it.
fn abandoned(store: &mut Store, owner: &str, path: &str, intended: &[u8]) -> u64 {
    store
        .transaction(true, |transaction| {
            let before = transaction.index().current_state(path);
            let object = transaction.object(Arc::from(intended), None)?;
            let after = State::File {
                object,
                length: intended.len() as u64,
                digest: crate::checkpoints::hash::digest(intended),
                mode: Mode::default(),
            };
            let seq = transaction.seq();
            transaction.record(Record::Prepare(Prepare {
                seq,
                owner: owner.into(),
                origin: "session-gone".into(),
                kind: "write".into(),
                path: path.into(),
                before,
                after,
                time: crate::sessions::now(),
            }));
            Ok(seq)
        })
        .unwrap()
}

#[test]
fn reopening_classifies_operations_left_by_exited_processes() {
    let fixture = Fixture::new();
    for name in ["applied.txt", "untouched.txt", "torn.txt", "live.txt"] {
        fixture.write(name, b"old\n");
    }
    let workspace = fixture.open();
    let location = super::super::identity::locate(fixture.home.path(), fixture.root.path(), false)
        .unwrap()
        .unwrap();
    // Another process that is still running.
    let mut live = Store::open(location).unwrap();
    let live_owner = live.owner.clone();
    let operations: Vec<u64> = {
        let mut store = workspace.store();
        ["applied.txt", "untouched.txt", "torn.txt"]
            .iter()
            .map(|path| abandoned(&mut store, "owner-exited", path, b"new\n"))
            .collect()
    };
    let running = abandoned(&mut live, &live_owner, "live.txt", b"new\n");
    fixture.write("applied.txt", b"new\n");
    fixture.write("torn.txt", b"ne");
    drop(workspace);

    let reopened = fixture.open();
    let index = index(&reopened);
    let outcomes: Vec<Outcome> = operations.iter().map(|seq| index.outcomes[seq]).collect();
    assert_eq!(
        outcomes,
        [Outcome::Applied, Outcome::NotApplied, Outcome::Unknown]
    );
    assert_eq!(index.pending.keys().copied().collect::<Vec<_>>(), [running]);
    let recovered = index
        .events
        .iter()
        .filter(|event| event.origin == "recovery")
        .count();
    assert_eq!(recovered, 3);
    assert_eq!(
        index.current("applied.txt").unwrap().operation,
        operations[0],
        "the applied result is credited to its operation"
    );

    // Every previous state stays restorable.
    for (seq, path) in operations
        .iter()
        .zip(["applied.txt", "untouched.txt", "torn.txt"])
    {
        let selected = paths(&[path]);
        reopened
            .restore(&request(&reopened, *seq, Some(&selected), Expected::Latest))
            .unwrap();
        assert_eq!(fixture.read(path).unwrap(), b"old\n", "{path}");
    }
    drop(live);
}
