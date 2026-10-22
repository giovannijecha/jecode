//! Integrated on-demand recovery contracts, including the legacy boundary.
use super::*;
use crate::cancel::Cancellation;
use crate::checkpoints::{recovery, scopes};

#[test]
fn directory_restore_includes_deleted_files_first_captured_after_group_creation() {
    let fixture = Fixture::new();
    fixture.write("src/deleted.txt", b"original\n");
    fixture.write("src/kept.txt", b"keep\n");
    fixture.write("outside.txt", b"unrelated\n");
    let workspace =
        workspace::open(fixture.home.path(), fixture.root.path(), Options::offline()).unwrap();
    assert!(index(&workspace).paths.is_empty());
    let captured = scopes::capture(
        &workspace,
        &paths(&["src/"]),
        "model",
        &Cancellation::default(),
    )
    .unwrap();
    let group = captured.group.unwrap();
    assert!(index(&workspace).at("src/deleted.txt", group).is_none());
    fs::remove_file(fixture.path("src/deleted.txt")).unwrap();
    fixture.write("outside.txt", b"external work\n");
    scopes::reconcile(&workspace, &captured, "external", &Cancellation::default()).unwrap();
    let selected = paths(&["src"]);
    let report = workspace
        .restore(&request(
            &workspace,
            group,
            Some(&selected),
            Expected::Latest,
        ))
        .unwrap();
    assert!(report.failed.is_none() && report.conflicts.is_empty());
    assert!(
        restored(&report)
            .iter()
            .any(|path| path == "src/deleted.txt")
    );
    assert_eq!(fixture.read("src/deleted.txt").unwrap(), b"original\n");
    assert_eq!(fixture.read("outside.txt").unwrap(), b"external work\n");
    assert!(index(&workspace).current("outside.txt").is_none());
}

#[test]
fn selected_recovery_restore_does_not_observe_unrelated_captured_file() {
    let fixture = Fixture::new();
    fixture.write("src/selected.txt", b"saved selected\n");
    fixture.write("src/sibling.txt", b"saved sibling\n");
    let workspace =
        workspace::open(fixture.home.path(), fixture.root.path(), Options::offline()).unwrap();
    let captured = scopes::capture(
        &workspace,
        &paths(&["src/"]),
        "model",
        &Cancellation::default(),
    )
    .unwrap();
    let group = captured.group.unwrap();
    let sibling_before = index(&workspace).current("src/sibling.txt").unwrap().seq;
    fixture.write("src/selected.txt", b"external selected\n");
    fixture.write("src/sibling.txt", b"external sibling\n");

    let selected = paths(&["src/selected.txt"]);
    let report = workspace
        .restore(&request(
            &workspace,
            group,
            Some(&selected),
            Expected::Latest,
        ))
        .unwrap();
    assert!(report.failed.is_none() && report.conflicts.is_empty());
    assert_eq!(
        fixture.read("src/selected.txt").unwrap(),
        b"saved selected\n"
    );
    assert_eq!(
        fixture.read("src/sibling.txt").unwrap(),
        b"external sibling\n"
    );
    assert_eq!(
        index(&workspace).current("src/sibling.txt").unwrap().seq,
        sibling_before
    );
}

#[test]
fn recursive_recovery_restore_observes_only_requested_subtree() {
    let fixture = Fixture::new();
    fixture.write("src/sub/deleted.txt", b"saved deleted\n");
    fixture.write("src/sibling.txt", b"saved sibling\n");
    let workspace =
        workspace::open(fixture.home.path(), fixture.root.path(), Options::offline()).unwrap();
    let captured = scopes::capture(
        &workspace,
        &paths(&["src/"]),
        "model",
        &Cancellation::default(),
    )
    .unwrap();
    let group = captured.group.unwrap();
    let sibling_before = index(&workspace).current("src/sibling.txt").unwrap().seq;
    fs::remove_file(fixture.path("src/sub/deleted.txt")).unwrap();
    fixture.write("src/sub/new.txt", b"new external\n");
    fixture.write("src/sibling.txt", b"external sibling\n");

    let selected = paths(&["src/sub"]);
    let report = workspace
        .restore(&request(
            &workspace,
            group,
            Some(&selected),
            Expected::Latest,
        ))
        .unwrap();
    assert!(report.failed.is_none() && report.conflicts.is_empty());
    assert_eq!(
        fixture.read("src/sub/deleted.txt").unwrap(),
        b"saved deleted\n"
    );
    assert!(fixture.read("src/sub/new.txt").is_none());
    assert_eq!(
        fixture.read("src/sibling.txt").unwrap(),
        b"external sibling\n"
    );
    assert_eq!(
        index(&workspace).current("src/sibling.txt").unwrap().seq,
        sibling_before
    );
}

#[test]
fn pinned_recovery_expectation_is_valid_below_the_legacy_horizon() {
    let fixture = Fixture::new();
    fixture.write("a.txt", b"saved\n");
    let workspace =
        workspace::open(fixture.home.path(), fixture.root.path(), Options::offline()).unwrap();
    let captured = scopes::capture(
        &workspace,
        &paths(&["a.txt"]),
        "model",
        &Cancellation::default(),
    )
    .unwrap();
    let group = captured.group.unwrap();
    workspace.pin(group, "user pin", true).unwrap();
    let mut store = workspace.store();
    // A targeted pin is independent of the legacy budget horizon. Construct
    // that valid index boundary directly; GC's pin copying has separate tests.
    store.index.horizon = store.index.latest();
    assert!(store.index.horizon > group && recovery::available(&store.index, group));
    let selected = paths(&["a.txt"]);
    let request = request(&workspace, group, Some(&selected), Expected::At(group));
    assert_eq!(
        crate::checkpoints::restore::paths(&mut store, &request).unwrap(),
        selected
    );
}

#[test]
fn inspecting_legacy_history_does_not_start_the_migration_grace_clock() {
    let fixture = Fixture::new();
    fixture.write("a.txt", b"before\n");
    let workspace = fixture.open();
    let target = index(&workspace).last_point().unwrap().seq;
    fixture.write("a.txt", b"after\n");
    let selected = paths(&["a.txt"]);
    let mut request = request(&workspace, target, Some(&selected), Expected::Latest);
    request.preview = true;
    let report = workspace.restore(&request).unwrap();
    assert_eq!(report.planned.len(), 1);
    assert!(index(&workspace).recovery.mode.is_none());
    assert_eq!(fixture.read("a.txt").unwrap(), b"after\n");
}

#[test]
fn failed_capture_retry_obeys_the_callers_cancellation_while_waiting_for_a_path() {
    use crate::checkpoints::workspace::Readiness;
    use std::time::Duration;
    let fixture = Fixture::new();
    fixture.write("a.txt", b"before\n");
    let workspace =
        workspace::open(fixture.home.path(), fixture.root.path(), Options::offline()).unwrap();
    workspace
        .signals
        .update(|health| health.readiness = Readiness::Failed("retryable fixture".into()));
    let held = workspace.leases.acquire(&["a.txt"]).unwrap();
    let cancellation = Cancellation::default();
    let child_token = cancellation.clone();
    let child_workspace = workspace.clone();
    let (sender, receiver) = std::sync::mpsc::channel();
    let worker = std::thread::spawn(move || {
        sender
            .send(child_workspace.wait_ready_cancellable(&child_token))
            .unwrap();
    });
    assert!(receiver.recv_timeout(Duration::from_millis(100)).is_err());
    cancellation.cancel();
    assert!(
        receiver
            .recv_timeout(Duration::from_secs(2))
            .unwrap()
            .is_err()
    );
    worker.join().unwrap();
    drop(held);
}
