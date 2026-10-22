use super::*;
use crate::checkpoints::gc;
use crate::checkpoints::identity::{self, Location};
use crate::checkpoints::journal;
use crate::checkpoints::quota;
use crate::checkpoints::record::{Discovery, Event, Outcome, Record, State, Version};
use crate::checkpoints::store::Store;
use std::sync::Arc;

const MIB: u64 = 1 << 20;

fn location(fixture: &Fixture) -> Location {
    identity::locate(fixture.home.path(), fixture.root.path(), false)
        .unwrap()
        .unwrap()
}

/// Eight distinct 200 KiB versions of one file: 1.6 MiB against a 1 MiB budget.
fn history(fixture: &Fixture, workspace: &Workspace) -> Vec<u64> {
    workspace
        .configure(Some(MIB), Some(512 << 10), "test")
        .unwrap();
    (0..8)
        .map(|seed| {
            fixture.write("big.bin", &noise(seed, 200 << 10));
            workspace
                .mark("milestone", &format!("v{seed}"), "test", false, false)
                .unwrap()
        })
        .collect()
}

#[test]
fn a_long_turn_releases_intermediate_versions_but_keeps_its_last_named_point() {
    let fixture = Fixture::new();
    fixture.write("long.bin", &noise(0, 80 << 10));
    let workspace = fixture.open();
    workspace
        .configure(Some(MIB), Some(128 << 10), "test")
        .unwrap();
    let baseline = *index(&workspace).points.keys().next_back().unwrap();
    for seed in 1..=20 {
        let bytes = noise(seed, 80 << 10);
        let mut mutation = workspace.mutation("test", "write", &fixture.path("long.bin"));
        mutation.prepare(&bytes);
        fixture.write("long.bin", &bytes);
        assert!(matches!(
            mutation.finish(true),
            crate::checkpoints::mutation::Protection::Protected { .. }
        ));
    }
    let collected = gc::collect(&location(&fixture), false).unwrap().unwrap();
    assert!(collected.horizon > baseline, "{collected:?}");
    assert!(collected.after < collected.before / 2, "{collected:?}");
    assert!(!collected.over_budget, "{collected:?}");
    assert!(index(&workspace).retained(baseline));
    let selected = paths(&["long.bin"]);
    workspace
        .restore(&request(
            &workspace,
            baseline,
            Some(&selected),
            Expected::Latest,
        ))
        .unwrap();
    assert_eq!(fixture.read("long.bin").unwrap(), noise(0, 80 << 10));
}

fn stored_version(store: &mut Store, seed: u64) -> Result<(), String> {
    let bytes: Arc<[u8]> = noise(seed, 80 << 10).into();
    store.transaction(true, |transaction| {
        let object = transaction.object(bytes.clone(), None)?;
        let seq = transaction.seq();
        transaction.record(Record::Version(Version {
            seq,
            path: "versions.bin".into(),
            state: State::File {
                object,
                length: bytes.len() as u64,
                digest: super::super::hash::digest(&bytes),
                mode: super::super::files::Mode(0),
            },
            stat: None,
            time: 0,
            origin: "test".into(),
            operation: 0,
            discovery: Discovery::Known,
        }));
        Ok(())
    })
}

#[test]
fn working_generation_frees_space_after_collection_and_unpin() {
    let fixture = Fixture::new();
    let workspace = fixture.open();
    workspace
        .configure(Some(MIB), Some(128 << 10), "test")
        .unwrap();
    let location = location(&fixture);
    let limit = MIB;
    let mut store = workspace.store();
    store.journal.quota_limit = limit;
    stored_version(&mut store, 1).unwrap();
    let pinned = store
        .transaction(true, |transaction| {
            Ok(transaction.point("milestone", "pinned", "test", true))
        })
        .unwrap();
    for seed in 2..=5 {
        stored_version(&mut store, seed).unwrap();
    }
    store
        .transaction(true, |transaction| {
            transaction.point("milestone", "latest", "test", false);
            Ok(())
        })
        .unwrap();
    let before = quota::measure(&location.directory, store.journal.generation).unwrap();
    assert!(before.generation < limit / 2);
    let error = stored_version(&mut store, 6).unwrap_err();
    assert!(quota::is_full(&error), "{error}");
    assert_eq!(
        quota::measure(&location.directory, store.journal.generation)
            .unwrap()
            .generation,
        before.generation
    );
    drop(store);

    let collected = gc::collect(&location, true).unwrap().unwrap();
    assert!(collected.after < collected.before, "{collected:?}");
    let mut store = workspace.store();
    store.maintain().unwrap();
    assert_eq!(store.journal.generation, collected.generation);
    store.journal.quota_limit = limit;
    stored_version(&mut store, 7).unwrap();
    let with_pin = store.index.object_bytes;
    drop(store);
    workspace.pin(pinned, "", false).unwrap();
    gc::collect(&location, true).unwrap().unwrap();
    let mut store = workspace.store();
    store.maintain().unwrap();
    assert!(store.index.object_bytes < with_pin);
    assert!(!store.index.retained(pinned));
}

#[test]
fn collection_keeps_pins_current_state_and_readers_within_the_budget() {
    let fixture = Fixture::new();
    fixture.write("small.txt", b"small\n");
    let workspace = fixture.open();
    let points = history(&fixture, &workspace);
    workspace.pin(points[1], "keep v1", true).unwrap();
    assert!(index(&workspace).object_bytes > MIB);

    let location = location(&fixture);
    let collection = gc::collect(&location, false).unwrap().unwrap();
    assert!(collection.after < collection.before, "{collection:?}");
    assert_eq!(collection.generation, 2);
    assert!(collection.horizon >= points[6], "{collection:?}");
    // This process still reads the old generation until it follows the move.
    let old = journal::generation_directory(&location.directory, 1);
    assert!(old.exists());
    let index = index(&workspace);
    assert!(!old.exists());
    assert_eq!(index.horizon, collection.horizon);
    assert!(index.object_bytes <= collection.after);

    let selected = paths(&["big.bin", "small.txt"]);
    for (point, seed) in [(points[1], 1), (*points.last().unwrap(), 7)] {
        let report = workspace
            .restore(&request(
                &workspace,
                point,
                Some(&selected),
                Expected::Latest,
            ))
            .unwrap();
        assert!(report.failed.is_none());
        assert_eq!(fixture.read("big.bin").unwrap(), noise(seed, 200 << 10));
        assert_eq!(fixture.read("small.txt").unwrap(), b"small\n");
    }
    let error = workspace
        .restore(&request(
            &workspace,
            points[2],
            Some(&selected),
            Expected::Latest,
        ))
        .err()
        .unwrap();
    assert!(error.contains("no longer retained"), "{error}");
    assert!(workspace.pin(points[3], "too late", true).is_err());

    // A new process sees the same retained history.
    drop(workspace);
    let reopened = fixture.open();
    let again = super::index(&reopened);
    assert_eq!(again.horizon, collection.horizon);
    assert!(again.pins.contains_key(&points[1]));
    assert!(
        gc::collect(&location, false).unwrap().is_none(),
        "nothing is due"
    );
}

#[test]
fn a_forced_collection_within_the_budget_keeps_every_point() {
    let fixture = Fixture::new();
    fixture.write("a.txt", b"one\n");
    let workspace = fixture.open();
    fixture.write("a.txt", b"two\n");
    let second = workspace
        .mark("milestone", "two", "test", false, false)
        .unwrap();
    let before = index(&workspace);
    let baseline = *before.points.keys().next().unwrap();
    let collection = gc::collect(&location(&fixture), true).unwrap().unwrap();
    assert_eq!(collection.horizon, baseline);
    let after = index(&workspace);
    assert!(
        before
            .points
            .keys()
            .all(|seq| after.points.contains_key(seq))
    );
    assert_eq!(after.paths["a.txt"].len(), before.paths["a.txt"].len());
    let selected = paths(&["a.txt"]);
    workspace
        .restore(&request(
            &workspace,
            baseline,
            Some(&selected),
            Expected::At(second),
        ))
        .unwrap();
    assert_eq!(fixture.read("a.txt").unwrap(), b"one\n");
}

#[test]
fn metadata_history_counts_toward_the_archive_budget_and_keeps_pins() {
    let fixture = Fixture::new();
    let workspace = fixture.open();
    workspace
        .configure(Some(MIB), Some(128 << 10), "test")
        .unwrap();
    let label = "metadata".repeat(16 << 10);
    let points: Vec<_> = (0..12)
        .map(|_| {
            workspace
                .mark("milestone", &label, "test", false, false)
                .unwrap()
        })
        .collect();
    workspace.pin(points[1], "keep", true).unwrap();
    let location = location(&fixture);
    let segment = workspace.store().journal.segment_path(1);
    let bytes = fs::metadata(segment).unwrap().len();
    assert!(bytes > MIB);
    assert_eq!(index(&workspace).object_bytes, 0);

    let collected = gc::collect(&location, false).unwrap().unwrap();
    assert_eq!(collected.before, bytes);
    assert!(collected.after < collected.before);
    assert!(collected.after <= MIB / 100 * 75);
    let after = index(&workspace);
    assert!(after.pins.contains_key(&points[1]));
    assert!(after.points.contains_key(points.last().unwrap()));
    assert!(!after.points.contains_key(&points[2]));
}

/// Compaction writes the move into the old journal before `CURRENT`; a
/// crash between the two is repaired by the next process that opens it.
#[test]
fn a_move_without_an_updated_current_file_is_followed_and_repaired() {
    let fixture = Fixture::new();
    fixture.write("small.txt", b"small\n");
    let workspace = fixture.open();
    let points = history(&fixture, &workspace);
    let location = location(&fixture);
    let collection = gc::collect(&location, false).unwrap().unwrap();
    let current = location.directory.join("CURRENT");
    fs::write(&current, b"1").unwrap();

    let store = Store::open(location.clone()).unwrap();
    assert_eq!(store.journal.generation, collection.generation);
    assert_eq!(store.index.horizon, collection.horizon);
    assert!(store.index.points.contains_key(points.last().unwrap()));
    assert_eq!(fs::read_to_string(&current).unwrap(), "2");
    drop(store);
    drop(workspace);
}

#[test]
fn a_published_but_missing_generation_keeps_the_previous_history() {
    let fixture = Fixture::new();
    fixture.write("a.txt", b"saved\n");
    let workspace = fixture.open();
    let location = location(&fixture);
    let old = journal::generation_directory(&location.directory, 1);
    drop(workspace);

    // Simulate a crash that preserved Moved but lost the new directory entry.
    let mut store = Store::open(location.clone()).unwrap();
    store
        .transaction(true, |transaction| {
            let seq = transaction.seq();
            transaction.record(Record::Moved { seq, generation: 2 });
            Ok(())
        })
        .unwrap();
    drop(store);
    let new = journal::generation_directory(&location.directory, 2);
    assert!(!new.exists());
    let error = Store::open(location.clone()).err().unwrap();
    assert!(error.contains("generation 2 is missing"), "{error}");
    assert!(old.exists() && !new.exists());

    // CURRENT can also survive while its generation directory does not.
    fs::write(location.directory.join("CURRENT"), b"2").unwrap();
    let error = Store::open(location).err().unwrap();
    assert!(error.contains("generation 2 is missing"), "{error}");
    assert!(old.exists() && !new.exists());
}

#[test]
fn journal_rejects_an_oversized_batch_before_writing_and_keeps_completion_space() {
    let fixture = Fixture::new();
    let workspace = fixture.open();
    let mut store = workspace.store();
    fs::write(
        store.workspace.directory.join("test-owned.bin"),
        vec![0; 300_000],
    )
    .unwrap();
    let before = quota::used(&store.workspace.directory).unwrap();
    let before_journal = store.journal.stored_bytes();
    store.journal.quota_limit = before + quota::RESERVE + 512;
    let error = store
        .transaction(false, |transaction| {
            let seq = transaction.seq();
            transaction.record(Record::Event(Event {
                seq,
                origin: "test".into(),
                time: 0,
                text: "x".repeat(1024),
                paths: Vec::new(),
            }));
            Ok(())
        })
        .unwrap_err();
    assert!(quota::is_full(&error), "{error}");
    assert_eq!(quota::used(&store.workspace.directory).unwrap(), before);

    store
        .transaction(false, |transaction| {
            let seq = transaction.seq();
            transaction.record(Record::Complete {
                seq,
                operation: 0,
                outcome: Outcome::Unknown,
            });
            Ok(())
        })
        .unwrap();
    let after = quota::used(&store.workspace.directory).unwrap();
    assert!(
        after > before,
        "before={before}, after={after}, journal={before_journal}->{}, segment={:?}",
        store.journal.stored_bytes(),
        fs::metadata(store.journal.segment_path(1)).map(|m| m.len())
    );
}
