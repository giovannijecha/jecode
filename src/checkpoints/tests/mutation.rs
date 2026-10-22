use super::*;
use crate::checkpoints::mutation::{Freshness, Protection};
use crate::checkpoints::record::{Outcome, Record, State};
use crate::checkpoints::workspace::Readiness;

fn protected(protection: Protection) -> (u64, u64) {
    match protection {
        Protection::Protected {
            before,
            after: Some(after),
        } => (before, after),
        other => panic!("expected a protected change, got {other:?}"),
    }
}

fn operation(index: &Index, snapshot: u64) -> u64 {
    *index
        .outcomes
        .range(snapshot..)
        .next()
        .expect("completed operation after snapshot")
        .0
}

#[test]
fn a_write_saves_the_previous_state_before_the_file_changes() {
    let fixture = Fixture::new();
    fixture.write("notes.txt", b"one\n");
    let workspace = fixture.open();
    // Changed outside any tool after the baseline, so not yet recorded.
    fixture.write("notes.txt", b"two\n");

    let mut mutation = workspace.mutation("session-a", "edit", &fixture.path("notes.txt"));
    assert_eq!(
        mutation.prepare_with_previous(b"three\n", 64),
        Some(Some("two\n".into()))
    );
    {
        // The intent is durable before the tool touches the file.
        let index = index(&workspace);
        let prepare = index.pending.values().next().unwrap();
        assert_eq!(
            (prepare.path.as_str(), prepare.origin.as_str()),
            ("notes.txt", "session-a")
        );
        assert_eq!(index.current("notes.txt").unwrap().origin, "external");
    }
    fixture.write("notes.txt", b"three\n");
    let (before, after) = protected(mutation.finish(true));

    let index = index(&workspace);
    assert!(index.pending.is_empty());
    let operation = operation(&index, before);
    assert_eq!(index.outcomes.get(&operation), Some(&Outcome::Applied));
    let current = index.current("notes.txt").unwrap();
    assert_eq!(
        (current.seq, current.origin.as_str(), current.operation),
        (after, "session-a", operation)
    );

    let selected = paths(&["notes.txt"]);
    workspace
        .restore(&request(
            &workspace,
            before,
            Some(&selected),
            Expected::At(after),
        ))
        .unwrap();
    assert_eq!(fixture.read("notes.txt").unwrap(), b"two\n");
}

#[test]
fn a_created_file_records_its_absence_so_restoring_removes_it() {
    let fixture = Fixture::new();
    fixture.write("kept.txt", b"kept\n");
    let workspace = fixture.open();
    let mut mutation = workspace.mutation("session-a", "write", &fixture.path("src/new.rs"));
    assert_eq!(
        mutation.prepare_with_previous(b"pub fn new() {}\n", 64),
        Some(None)
    );
    fixture.write("src/new.rs", b"pub fn new() {}\n");
    let (before, _) = protected(mutation.finish(true));
    assert_eq!(
        index(&workspace).state_at("src/new.rs", before),
        State::Absent
    );

    let selected = paths(&["src/new.rs"]);
    let report = workspace
        .restore(&request(
            &workspace,
            before,
            Some(&selected),
            Expected::Latest,
        ))
        .unwrap();
    assert_eq!(restored(&report), ["src/new.rs"]);
    assert!(fixture.read("src/new.rs").is_none());
    assert_eq!(fixture.read("kept.txt").unwrap(), b"kept\n");
}

#[test]
fn failed_and_abandoned_writes_are_classified_from_the_actual_file() {
    let fixture = Fixture::new();
    fixture.write("a.txt", b"a\n");
    let workspace = fixture.open();

    let mut failed = workspace.mutation("session-a", "write", &fixture.path("a.txt"));
    failed.prepare(b"never written\n");
    let (operation, _) = protected(failed.finish(false));
    assert_eq!(
        index(&workspace)
            .outcomes
            .range(operation..)
            .next()
            .map(|(_, outcome)| outcome),
        Some(&Outcome::NotApplied)
    );

    // A tool that returns early without finishing releases the path on drop.
    let mut abandoned = workspace.mutation("session-a", "write", &fixture.path("a.txt"));
    abandoned.prepare(b"partial\n");
    fixture.write("a.txt", b"part");
    drop(abandoned);
    let index = index(&workspace);
    assert!(index.pending.is_empty());
    assert_eq!(index.outcomes.values().last(), Some(&Outcome::Unknown));
    assert!(matches!(
        index.current_state("a.txt"),
        State::File { length: 4, .. }
    ));
}

#[test]
fn a_finished_write_is_recorded_from_its_bytes_and_reread_before_the_next_point() {
    let fixture = Fixture::new();
    fixture.write("a.txt", b"one\n");
    let workspace = fixture.open();

    let mut mutation = workspace.mutation("session-a", "write", &fixture.path("a.txt"));
    mutation.prepare(b"two\n");
    let reused_before_finish = workspace.store().counters.objects_reused;
    fixture.write("a.txt", b"two\n");
    // Another writer keeps the length before the tool finishes.
    fixture.write("a.txt", b"six\n");
    let (operation, after) = protected(mutation.finish(true));
    assert_eq!(
        workspace.store().counters.objects_reused,
        reused_before_finish
    );
    assert_eq!(
        index(&workspace)
            .outcomes
            .range(operation..)
            .next()
            .map(|(_, outcome)| outcome),
        Some(&Outcome::Applied)
    );

    workspace
        .mark("turn", "next", "session-a", false, false)
        .unwrap();
    let index = index(&workspace);
    let current = index.current("a.txt").unwrap();
    assert!(current.seq > after);
    assert!(matches!(
        &current.state,
        State::File { digest, .. } if *digest == crate::checkpoints::hash::digest(b"six\n")
    ));
}

#[test]
fn a_full_working_generation_closes_a_prepared_write_as_unknown_and_reconciles_it_later() {
    let fixture = Fixture::new();
    fixture.write("a.txt", b"one\n");
    fixture.write("padding.bin", &noise(31, 300 * 1024));
    let workspace = fixture.open();
    let mut mutation = workspace.mutation("session-a", "write", &fixture.path("a.txt"));
    mutation.prepare(b"two\n");
    let mut store = workspace.store();
    let used =
        crate::checkpoints::quota::measure(&store.workspace.directory, store.journal.generation)
            .unwrap();
    let completion = Record::Complete {
        seq: 0,
        operation: 0,
        outcome: Outcome::Unknown,
    }
    .frame()
    .len() as u64;
    store.journal.quota_limit = 2 * (used.generation + completion + 1);
    drop(store);
    fixture.write("a.txt", b"two\n");
    let protection = mutation.finish(true);
    let operation = match protection {
        Protection::Protected {
            before,
            after: None,
        } => before,
        other => panic!("expected saved preimage without an after ref, got {other:?}"),
    };
    let snapshot = index(&workspace);
    assert!(snapshot.pending.is_empty());
    assert_eq!(
        snapshot
            .outcomes
            .range(operation..)
            .next()
            .map(|(_, outcome)| outcome),
        Some(&Outcome::Unknown)
    );

    workspace.store().journal.quota_limit = crate::checkpoints::quota::LIMIT;
    workspace
        .mark("turn", "reconcile", "session-a", false, false)
        .unwrap();
    let State::File { object, .. } = index(&workspace).current_state("a.txt") else {
        panic!("the full archive's write was not reconciled");
    };
    assert_eq!(&workspace.store().bytes(object).unwrap()[..], b"two\n");
}

#[test]
fn unprotected_changes_still_proceed_and_say_why() {
    let fixture = Fixture::new();
    fixture.write("node_modules/pkg/index.js", b"x\n");
    fixture.write("ok.txt", b"ok\n");
    let workspace = fixture.open();
    workspace.configure(None, Some(1024), "test").unwrap();
    fixture.write("big.bin", &noise(3, 2048));
    workspace
        .mark("milestone", "limits", "test", false, false)
        .unwrap();

    let outside = Directory::new();
    let cases = [
        (fixture.path("node_modules/pkg/index.js"), "excluded"),
        (outside.path().join("other.txt"), "outside"),
    ];
    for (path, expected) in cases {
        let mut mutation = workspace.mutation("session-a", "write", &path);
        mutation.prepare(b"y\n");
        match mutation.finish(true) {
            Protection::Excluded(reason) => assert!(reason.contains(expected), "{reason}"),
            other => panic!("{other:?}"),
        }
    }

    let mut oversized = workspace.mutation("session-a", "write", &fixture.path("big.bin"));
    oversized.prepare(b"small now\n");
    fixture.write("big.bin", b"small now\n");
    match oversized.finish(true) {
        Protection::Unprotected(reason) => assert!(reason.contains("not captured"), "{reason}"),
        other => panic!("{other:?}"),
    }

    // A failing archive never blocks the tool; it only loses protection.
    workspace.store().journal.fault = Some("synthetic disk failure".into());
    let mut mutation = workspace.mutation("session-a", "write", &fixture.path("ok.txt"));
    mutation.prepare(b"changed\n");
    fixture.write("ok.txt", b"changed\n");
    match mutation.finish(true) {
        Protection::Unprotected(reason) => {
            assert!(reason.contains("synthetic disk failure"), "{reason}")
        }
        other => panic!("{other:?}"),
    }
    workspace.store().journal.fault = None;
    workspace
        .signals
        .update(|health| health.readiness = Readiness::Failed("synthetic failure".into()));
    let mut mutation = workspace.mutation("session-a", "write", &fixture.path("ok.txt"));
    mutation.prepare(b"still protected\n");
    assert!(
        matches!(mutation.finish(false), Protection::Protected { .. }),
        "a background capture failure does not disable exact-path protection"
    );
    // Explicit scope protection is independent of legacy baseline readiness.
    let command = workspace.command("session-a", "true");
    assert_eq!(command.freshness, Freshness::Current);
}

#[test]
fn commands_record_a_point_before_and_report_what_changed() {
    let fixture = Fixture::new();
    fixture.write("a.txt", b"a\n");
    fixture.write("b.txt", b"b\n");
    let workspace = fixture.open();

    let command = workspace.command("session-a", "cargo fmt");
    assert_eq!(command.freshness, Freshness::Current);
    let before = command.point.clone().unwrap();
    assert_eq!(workspace.signals.lock().bash, ["session-a"]);
    fixture.write("a.txt", b"formatted\n");
    fs::remove_file(fixture.path("b.txt")).unwrap();
    fixture.write("c.txt", b"c\n");
    let report = command.finish();
    assert!(report.error.is_none(), "{:?}", report.error);
    assert_eq!(report.before, Some(before));
    assert_eq!(report.changed, ["a.txt", "b.txt", "c.txt"]);
    assert!(workspace.signals.lock().bash.is_empty());
    let index = index(&workspace);
    assert_eq!(index.current("a.txt").unwrap().origin, "external");

    // Each protected command has its own before-state inside the same attempt.
    let first = workspace.command("session-a", "ls").finish();
    let second = workspace.command("session-a", "ls").finish();
    assert!(first.changed.is_empty() && second.changed.is_empty());
    assert_eq!(first.group, second.group);
    assert_ne!(first.before, second.before);

    // Undoing the command restores every path it touched.
    let report = workspace
        .restore(&request(&workspace, before, None, Expected::Latest))
        .unwrap();
    assert!(report.failed.is_none() && report.conflicts.is_empty());
    assert_eq!(fixture.read("a.txt").unwrap(), b"a\n");
    assert_eq!(fixture.read("b.txt").unwrap(), b"b\n");
    assert!(fixture.read("c.txt").is_none());
}

#[test]
fn a_failed_notification_capture_is_reconciled_without_another_event() {
    let fixture = Fixture::new();
    fixture.write("a.txt", b"before\n");
    let workspace = fixture.open();
    fixture.write("a.txt", b"after\n");
    workspace.signals.update(|health| {
        health.dirty.insert("a.txt".into());
    });
    workspace.store().journal.fault = Some("synthetic disk failure".into());
    assert!(workspace.flush("external").is_err());
    assert!(workspace.signals.lock().rescan.is_some());
    workspace.store().journal.fault = None;
    workspace.flush("external").unwrap();
    let State::File { object, .. } = index(&workspace).current_state("a.txt") else {
        panic!("the changed file was not captured");
    };
    assert_eq!(&workspace.store().bytes(object).unwrap()[..], b"after\n");
}

#[test]
fn a_failed_baseline_recovers_before_the_next_point() {
    let fixture = Fixture::new();
    fixture.write("a.txt", b"saved\n");
    let workspace =
        workspace::open(fixture.home.path(), fixture.root.path(), Options::offline()).unwrap();
    workspace.store().journal.fault = Some("synthetic disk failure".into());
    assert!(workspace.initialize().is_err());
    assert!(matches!(workspace.readiness(), Readiness::Failed(_)));
    workspace.store().journal.fault = None;
    let point = workspace
        .mark("turn", "recovered", "test", false, false)
        .unwrap();
    assert_eq!(workspace.readiness(), Readiness::Ready);
    assert!(index(&workspace).coverage_at(point));
    assert!(matches!(
        index(&workspace).current_state("a.txt"),
        State::File { .. }
    ));
}

#[test]
fn background_counts_from_two_sessions_do_not_overwrite_each_other() {
    let fixture = Fixture::new();
    let workspace = fixture.open();
    workspace.set_background(1, 1);
    workspace.set_background(2, 1);
    assert_eq!(
        workspace.signals.lock().background.values().sum::<usize>(),
        2
    );
    workspace.set_background(1, 0);
    assert_eq!(
        workspace.signals.lock().background.values().sum::<usize>(),
        1
    );
    workspace.set_background(2, 0);
    assert!(workspace.signals.lock().background.is_empty());
}

#[test]
fn scan_never_reads_a_recorded_child_through_a_linked_parent() {
    let fixture = Fixture::new();
    fixture.write("dir/file.txt", b"inside\n");
    let outside = Directory::new();
    fs::write(outside.path().join("file.txt"), b"private outside bytes\n").unwrap();
    let workspace = fixture.open();
    fs::remove_file(fixture.path("dir/file.txt")).unwrap();
    fs::remove_dir(fixture.path("dir")).unwrap();
    if crate::checkpoints::files::symlink(&outside.path().to_string_lossy(), &fixture.path("dir"))
        .is_err()
    {
        return;
    }
    workspace
        .mark("turn", "after link", "test", false, false)
        .unwrap();
    assert!(matches!(
        index(&workspace).current_state("dir/file.txt"),
        State::Unreadable { .. }
    ));
}

#[test]
fn child_of_a_directory_replaced_by_a_file_is_absent() {
    let fixture = Fixture::new();
    fixture.write("dir/child.txt", b"old\n");
    let workspace = fixture.open();
    fs::remove_file(fixture.path("dir/child.txt")).unwrap();
    fs::remove_dir(fixture.path("dir")).unwrap();
    fixture.write("dir", b"new file\n");
    workspace
        .mark("turn", "parent is file", "test", false, false)
        .unwrap();
    assert_eq!(
        index(&workspace).current_state("dir/child.txt"),
        State::Absent
    );
}

#[cfg(unix)]
#[test]
fn unreadable_directory_does_not_delete_recorded_children() {
    use std::os::unix::fs::PermissionsExt;

    let fixture = Fixture::new();
    fixture.write("dir/file.txt", b"saved\n");
    let workspace = fixture.open();
    let directory = fixture.path("dir");
    fs::set_permissions(&directory, fs::Permissions::from_mode(0o000)).unwrap();
    if fs::read_dir(&directory).is_ok() {
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o700)).unwrap();
        return; // Privileged users can enumerate despite the mode.
    }
    let result = workspace.mark("turn", "unreadable", "test", false, false);
    fs::set_permissions(&directory, fs::Permissions::from_mode(0o700)).unwrap();
    result.unwrap();
    assert!(matches!(
        index(&workspace).current_state("dir/file.txt"),
        State::File { .. }
    ));
}

/// An edit that keeps the size and restores the modification time is found
/// by the change time on Unix, and by rereading on reopen everywhere.
#[test]
fn same_size_edits_with_a_preserved_modification_time_are_captured() {
    let fixture = Fixture::new();
    fixture.write("same.txt", b"aaaa\n");
    let workspace = fixture.open();
    let preserve = |bytes: &[u8]| {
        let path = fixture.path("same.txt");
        let modified = fs::metadata(&path).unwrap().modified().unwrap();
        fs::write(&path, bytes).unwrap();
        fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(modified)
            .unwrap();
    };
    #[cfg(unix)]
    {
        preserve(b"bbbb\n");
        workspace
            .mark("milestone", "edited", "test", false, false)
            .unwrap();
        let index = index(&workspace);
        let State::File { object, .. } = index.current_state("same.txt") else {
            panic!()
        };
        assert_eq!(&workspace.store().bytes(object).unwrap()[..], b"bbbb\n");
    }
    drop(workspace);
    preserve(b"cccc\n");
    let reopened = fixture.open_with(Options {
        verify: true,
        ..Options::offline()
    });
    let State::File { object, .. } = index(&reopened).current_state("same.txt") else {
        panic!()
    };
    assert_eq!(&reopened.store().bytes(object).unwrap()[..], b"cccc\n");
}
