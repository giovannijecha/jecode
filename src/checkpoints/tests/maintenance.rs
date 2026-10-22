use super::*;
use crate::checkpoints::gc;
use crate::checkpoints::identity;
use crate::checkpoints::journal;
use crate::checkpoints::store::Store;
use crate::checkpoints::workspace::Readiness;
use std::time::{Duration, Instant};

fn wait_until(mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !condition() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(25));
    }
    assert!(condition(), "the idle archive reader did not refresh");
}

#[test]
fn idle_reader_follows_a_move_and_releases_the_old_generation() {
    let fixture = Fixture::new();
    fixture.write("source.rs", b"saved\n");
    let workspace = fixture.open_with(Options {
        watch: false,
        background: true,
        verify: false,
    });
    wait_until(|| workspace.readiness() == Readiness::Ready);
    let location = identity::locate(fixture.home.path(), fixture.root.path(), false)
        .unwrap()
        .unwrap();
    // A second owner holds the same generation, as a separate process would.
    let mut peer = Store::open(location.clone()).unwrap();
    let old = journal::generation_directory(&location.directory, peer.journal.generation);
    let moved = gc::collect(&location, true).unwrap().unwrap();
    assert!(old.exists());

    // No workspace operation is issued. Its worker must notice the move.
    wait_until(|| workspace.store().journal.generation == moved.generation);
    assert!(old.exists(), "the peer still reads the old generation");
    peer.maintain().unwrap();
    assert_eq!(peer.journal.generation, moved.generation);
    assert!(!old.exists(), "both readers released the old generation");
}

#[test]
fn a_new_point_below_the_trigger_does_not_repeat_collection() {
    let fixture = Fixture::new();
    // Baseline this file under the normal budget. The small test budget below
    // intentionally limits discovery of *new* large paths.
    fixture.write("source.rs", &noise(0, 350 << 10));
    let workspace = fixture.open();
    workspace
        .configure(Some(1 << 20), Some(512 << 10), "test")
        .unwrap();
    fixture.write("source.rs", &noise(1, 350 << 10));
    workspace
        .mark("milestone", "version 1", "test", false, false)
        .unwrap();
    fixture.write("source.rs", &noise(2, 350 << 10));
    workspace.verify().unwrap();
    let first = workspace.collect(false).unwrap().unwrap();
    assert!(first.after < first.before, "{first:?}");

    workspace
        .mark("milestone", "current version", "test", false, false)
        .unwrap();
    let before = workspace.store().journal.stored_bytes();
    assert!(before < first.after + (1 << 20) / 10);
    workspace.maintain();
    let second = workspace
        .signals
        .lock()
        .collection
        .clone()
        .unwrap()
        .unwrap();
    assert_eq!(second.generation, first.generation);
    assert!(workspace.store().journal.stored_bytes() < first.after + (1 << 20) / 10);
}

#[test]
fn unpin_retries_collection_without_tenth_budget_growth() {
    let fixture = Fixture::new();
    fixture.write("source.rs", &noise(0, 350 << 10));
    let workspace = fixture.open();
    workspace
        .configure(Some(1 << 20), Some(512 << 10), "test")
        .unwrap();
    let points: Vec<_> = (1..4)
        .map(|seed| {
            fixture.write("source.rs", &noise(seed, 350 << 10));
            workspace
                .mark(
                    "milestone",
                    &format!("version {seed}"),
                    "test",
                    false,
                    false,
                )
                .unwrap()
        })
        .collect();
    workspace.pin(points[0], "old", true).unwrap();
    workspace.pin(points[1], "middle", true).unwrap();
    let _ = workspace.collect(false).unwrap();
    let (before, generation) = {
        let store = workspace.store();
        (store.journal.stored_bytes(), store.journal.generation)
    };

    workspace.pin(points[1], "middle", false).unwrap();
    assert!(workspace.store().journal.stored_bytes() < before + (1 << 20) / 10);
    workspace.maintain();
    let store = workspace.store();
    assert!(store.journal.generation > generation);
    assert!(store.journal.stored_bytes() < before);
    assert!(store.index.pins.contains_key(&points[0]));
    assert!(!store.index.pins.contains_key(&points[1]));
}
