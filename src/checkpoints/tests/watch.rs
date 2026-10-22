//! The Windows notification helper, driven through real file changes.
use super::*;
use crate::checkpoints::record::State;
use crate::checkpoints::workspace::{Readiness, Watching};
use std::time::{Duration, Instant};

fn until(what: &str, mut done: impl FnMut() -> bool) {
    let started = Instant::now();
    while !done() {
        assert!(
            started.elapsed() < Duration::from_secs(60),
            "timed out waiting for {what}"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Rewrites a file with content of the same size and its old modification time.
fn disguise(fixture: &Fixture, relative: &str, bytes: &[u8]) {
    let path = fixture.path(relative);
    let modified = fs::metadata(&path).unwrap().modified().unwrap();
    fs::write(&path, bytes).unwrap();
    fs::File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_modified(modified)
        .unwrap();
}

fn content(workspace: &Workspace, relative: &str) -> Vec<u8> {
    let State::File { object, .. } = index(workspace).current_state(relative) else {
        return Vec::new();
    };
    workspace.store().bytes(object).unwrap().to_vec()
}

#[test]
fn ordinary_points_drain_reported_paths_without_waiting_for_quiet() {
    let fixture = Fixture::new();
    fixture.write("a.txt", b"before\n");
    let workspace = fixture.open_with(Options::interactive());
    workspace.initialize().unwrap();
    until("the baseline", || workspace.readiness() == Readiness::Ready);
    assert_eq!(workspace.watching(), Watching::Active);

    fixture.write("a.txt", b"after\n");
    workspace.signals.update(|health| {
        health.dirty.insert("a.txt".into());
        // A point must not wait for the quiet-window deadline. The timestamp
        // is deliberately ahead so the old settle path reaches its 300 ms cap.
        health.event = Some(Instant::now() + Duration::from_secs(1));
    });
    let markers = || {
        workspace
            .watcher
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .markers_sent()
    };
    let before = markers();
    let point = workspace
        .mark("turn", "after", "test", false, false)
        .unwrap();
    assert_eq!(
        markers(),
        before,
        "an ordinary point requested a notification quiet window"
    );
    assert_eq!(content(&workspace, "a.txt"), b"after\n");
    assert!(index(&workspace).coverage_at(point));
}

#[test]
fn notification_queue_overflow_requests_one_rescan_and_stops_growing() {
    let fixture = Fixture::new();
    let workspace = fixture.open();
    let mut health = workspace.signals.lock();
    for number in 0..4096 {
        super::super::watch::queue_path(&mut health, &format!("dir\\file{number:04}.txt"));
    }
    super::super::watch::queue_path(&mut health, "dir/file0000.txt");
    assert_eq!(health.dirty.len(), 4096, "duplicates do not overflow");
    assert!(health.rescan.is_none());

    super::super::watch::queue_path(&mut health, "another.txt");
    assert!(health.dirty.is_empty());
    assert!(health.rescan.as_deref().unwrap().contains("4096"));
    super::super::watch::queue_path(&mut health, "later.txt");
    assert!(
        health.dirty.is_empty(),
        "the pending scan absorbs later paths"
    );
}

#[test]
fn excluded_notifications_do_not_fill_the_queue_or_extend_quiet() {
    let fixture = Fixture::new();
    let workspace = fixture.open();
    let mut health = workspace.signals.lock();
    health.excluded.push("private/archive".into());
    let previous = Instant::now() - Duration::from_secs(1);
    health.event = Some(previous);
    for number in 0..5000 {
        super::super::watch::queue_path(&mut health, &format!("target/build/f{number}.obj"));
        super::super::watch::queue_path(&mut health, &format!("node_modules/pkg/f{number}.js"));
        super::super::watch::queue_path(&mut health, &format!("private/archive/f{number}"));
    }
    assert!(health.dirty.is_empty());
    assert!(health.rescan.is_none());
    assert_eq!(health.event, Some(previous));
}

#[test]
fn scope_overrides_and_ambiguous_directory_names_keep_notifications() {
    let fixture = Fixture::new();
    let workspace = fixture.open();
    let mut health = workspace.signals.lock();
    health.policy.include.push("target".into());
    super::super::watch::queue_path(&mut health, "target\\generated\\file.rs");
    assert!(health.dirty.contains("target/generated/file.rs"));

    // A watcher path does not say whether it is a file or directory.
    health.policy.exclude.push("output/".into());
    let earlier = health.event;
    super::super::watch::queue_path(&mut health, "output/child.rs");
    assert!(!health.dirty.contains("output/child.rs"));
    assert_eq!(health.event, earlier);
    super::super::watch::queue_path(&mut health, "output");
    assert!(health.dirty.contains("output"));

    health.policy.exclude.push("special".into());
    health.policy.include.push("special/".into());
    super::super::watch::queue_path(&mut health, "special");
    assert!(health.dirty.contains("special"));
    super::super::watch::queue_path(&mut health, "special/child.rs");
    assert!(health.dirty.contains("special/child.rs"));
}

#[test]
fn notifications_capture_disguised_edits_and_a_lost_helper_forces_a_verified_rescan() {
    let fixture = Fixture::new();
    fixture.write("same.txt", b"aaaa\n");
    let workspace = fixture.open_with(Options::interactive());
    workspace.initialize().unwrap();
    until("the baseline", || workspace.readiness() == Readiness::Ready);
    assert_eq!(workspace.watching(), Watching::Active);

    // Stat alone cannot see this edit on Windows; the notification does.
    disguise(&fixture, "same.txt", b"bbbb\n");
    until("the notification", || {
        content(&workspace, "same.txt") == b"bbbb\n"
    });

    // Changes while the helper is gone are found by the forced rescan.
    workspace.watcher.lock().unwrap().as_mut().unwrap().kill();
    disguise(&fixture, "same.txt", b"cccc\n");
    until("the restart", || workspace.signals.lock().restarts > 0);
    until("the rescan", || {
        content(&workspace, "same.txt") == b"cccc\n"
    });
    until("the restarted helper", || {
        workspace.watching() == Watching::Active
    });
    // The helper can be ready and the content batch visible before the
    // verified rescan records its final complete-coverage point.
    until("the completed verified rescan", || {
        let index = index(&workspace);
        index.coverage_complete()
            && index
                .coverage
                .values()
                .any(|(complete, reason)| !complete && reason.contains("restarted"))
    });

    // An overflow is handled the same way.
    disguise(&fixture, "same.txt", b"dddd\n");
    workspace.signals.update(|health| {
        health.rescan = Some("change notifications overflowed".into());
    });
    until("the overflow rescan", || {
        content(&workspace, "same.txt") == b"dddd\n"
    });
}

#[test]
fn a_helper_that_keeps_failing_falls_back_to_scans_at_each_point() {
    let fixture = Fixture::new();
    fixture.write("a.txt", b"one\n");
    let workspace = fixture.open_with(Options::interactive());
    workspace.initialize().unwrap();
    until("the baseline", || workspace.readiness() == Readiness::Ready);
    let stopped = |workspace: &Workspace| matches!(workspace.watching(), Watching::Unavailable(reason) if reason.contains("stopped repeatedly"));
    for attempt in 1.. {
        until("a settled helper", || {
            workspace.watching() == Watching::Active || stopped(&workspace)
        });
        if stopped(&workspace) {
            break;
        }
        assert!(attempt <= 10, "the helper restarted without limit");
        if let Some(watcher) = workspace.watcher.lock().unwrap().as_mut() {
            watcher.kill();
        }
        until("the loss", || workspace.signals.lock().restarts >= attempt);
    }
    // Without notifications each point scans first.
    fixture.write("a.txt", b"three\n");
    let point = workspace
        .mark("turn", "after", "test", false, true)
        .unwrap();
    assert_eq!(content(&workspace, "a.txt"), b"three\n");
    assert!(index(&workspace).coverage_at(point));
}

#[test]
fn scope_rules_edited_by_another_process_rescan_before_the_next_point() {
    let fixture = Fixture::new();
    fixture.write("node_modules/pkg/index.js", b"dependency\n");
    let workspace = fixture.open_with(Options::interactive());
    workspace.initialize().unwrap();
    until("the baseline", || workspace.readiness() == Readiness::Ready);
    assert_eq!(workspace.watching(), Watching::Active);

    // No notification reports a file that only came into scope.
    let archive = workspace.archive.clone();
    let mut policy = crate::checkpoints::policy::Policy::read(&archive)
        .unwrap()
        .unwrap();
    policy.include.push("node_modules".into());
    policy.save(&archive).unwrap();
    let location = workspace.store().workspace.clone();
    drop(crate::checkpoints::store::Store::open(location).unwrap());
    workspace
        .mark("turn", "after", "test", false, true)
        .unwrap();
    assert_eq!(
        content(&workspace, "node_modules/pkg/index.js"),
        b"dependency\n"
    );
}

fn absent(workspace: &Workspace, relative: &str) -> bool {
    index(workspace).current_state(relative) == State::Absent
}

#[test]
fn notifications_record_creates_deletes_renames_and_whole_subtrees() {
    let fixture = Fixture::new();
    fixture.write("keep.txt", b"keep\n");
    fixture.write("old.txt", b"old\n");
    let workspace = fixture.open_with(Options::interactive());
    workspace.initialize().unwrap();
    until("the baseline", || workspace.readiness() == Readiness::Ready);
    assert_eq!(workspace.watching(), Watching::Active);

    fixture.write("new.txt", b"new\n");
    until("the create", || content(&workspace, "new.txt") == b"new\n");
    fs::remove_file(fixture.path("new.txt")).unwrap();
    until("the delete", || absent(&workspace, "new.txt"));
    fs::rename(fixture.path("old.txt"), fixture.path("renamed.txt")).unwrap();
    until("the rename", || {
        absent(&workspace, "old.txt") && content(&workspace, "renamed.txt") == b"old\n"
    });

    // A directory created, renamed and removed as a whole.
    fixture.write("tree/inner/deep.txt", b"deep\n");
    until("the subtree", || {
        content(&workspace, "tree/inner/deep.txt") == b"deep\n"
    });
    fs::rename(fixture.path("tree"), fixture.path("moved")).unwrap();
    until("the subtree rename", || {
        absent(&workspace, "tree/inner/deep.txt")
            && content(&workspace, "moved/inner/deep.txt") == b"deep\n"
    });
    fs::remove_dir_all(fixture.path("moved")).unwrap();
    until("the subtree delete", || {
        absent(&workspace, "moved/inner/deep.txt") && absent(&workspace, "moved")
    });
    assert_eq!(content(&workspace, "keep.txt"), b"keep\n");
    assert_eq!(workspace.watching(), Watching::Active);
}
