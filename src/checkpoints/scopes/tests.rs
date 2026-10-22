use super::*;
use crate::checkpoints::tests::{Fixture, index};
use crate::checkpoints::workspace::{self, Options};
use std::fs;

fn workspace(fixture: &Fixture) -> Arc<Workspace> {
    workspace::open(fixture.home.path(), fixture.root.path(), Options::offline()).unwrap()
}

fn capture_paths(workspace: &Arc<Workspace>, paths: &[&str]) -> Captured {
    capture(
        workspace,
        &paths.iter().map(|path| (*path).into()).collect::<Vec<_>>(),
        "test",
        &Cancellation::default(),
    )
    .unwrap()
}

#[test]
#[ignore = "explicit release measurement of scoped capture and reconciliation"]
fn measure_declared_directory_work() {
    use std::time::Instant;
    println!("paths,sample,capture_ms,reconcile_ms");
    for count in [100, 1000] {
        let fixture = Fixture::new();
        for number in 0..count {
            fixture.write(&format!("bulk/{number:04}.txt"), b"old");
        }
        let workspace = workspace(&fixture);
        let first = capture_paths(&workspace, &["bulk/"]);
        assert!(first.partial.is_empty());
        for sample in 0..12 {
            let started = Instant::now();
            let captured = capture_paths(&workspace, &["bulk/"]);
            let capture_ms = started.elapsed().as_secs_f64() * 1000.0;
            assert!(captured.partial.is_empty());
            let content = format!("revision-{sample}");
            for number in 0..count {
                fixture.write(&format!("bulk/{number:04}.txt"), content.as_bytes());
            }
            let started = Instant::now();
            let report =
                reconcile(&workspace, &captured, "external", &Cancellation::default()).unwrap();
            let reconcile_ms = started.elapsed().as_secs_f64() * 1000.0;
            assert!(report.partial.is_empty());
            assert_eq!(report.observed, count + 1);
            let store = workspace.store();
            for number in 0..count {
                let state = store.index.current_state(&format!("bulk/{number:04}.txt"));
                let State::File { object, .. } = state else {
                    panic!("{state:?}")
                };
                assert_eq!(&*store.bytes(object).unwrap(), content.as_bytes());
            }
            if sample >= 2 {
                println!("{count},{},{capture_ms:.6},{reconcile_ms:.6}", sample - 2);
            }
        }
    }
}

#[test]
fn exact_file_and_absence_are_saved_without_a_workspace_baseline() {
    let fixture = Fixture::new();
    fixture.write("existing.txt", b"before");
    let workspace = workspace(&fixture);
    let saved = capture_paths(&workspace, &["existing.txt", "new.txt"]);
    assert_eq!(saved.saved, 2, "{:?}", saved.partial);
    assert!(saved.partial.is_empty());
    let before = saved.snapshot.unwrap();
    let index = index(&workspace);
    assert!(matches!(
        super::super::recovery::target(&index, before, "existing.txt"),
        Some(State::File { .. })
    ));
    assert_eq!(
        super::super::recovery::target(&index, before, "new.txt"),
        Some(State::Absent)
    );
    assert!(index.current("unrelated.txt").is_none());
}

#[test]
fn an_empty_scope_does_not_wait_for_the_archive() {
    let fixture = Fixture::new();
    let workspace = workspace(&fixture);
    let held = workspace.store();
    let (sent, received) = std::sync::mpsc::channel();
    let worker = Arc::clone(&workspace);
    let thread = std::thread::spawn(move || {
        let result = capture(&worker, &[], "test", &Cancellation::default());
        sent.send(result).unwrap();
    });
    let result = received.recv_timeout(std::time::Duration::from_secs(1));
    drop(held);
    thread.join().unwrap();
    let captured = result
        .expect("no protection was requested, but capture waited for the store")
        .unwrap();
    assert!(captured.snapshot.is_none());
    assert!(captured.selection.files.is_empty() && captured.selection.directories.is_empty());
}

#[test]
fn a_narrow_scope_keeps_deleted_children_without_sibling_prefixes() {
    let fixture = Fixture::new();
    for path in [
        "source/keep.txt",
        "source/deleted.txt",
        "source.c",
        "source2/deleted.txt",
    ] {
        fixture.write(path, b"before");
    }
    let workspace = workspace(&fixture);
    capture_paths(&workspace, &["."]);
    fs::remove_file(fixture.path("source/deleted.txt")).unwrap();
    let captured = capture_paths(&workspace, &["source/"]);
    assert!(captured.partial.is_empty());
    let index = index(&workspace);
    let before = captured.snapshot.unwrap();
    assert_eq!(
        super::super::recovery::target(&index, before, "source/deleted.txt"),
        Some(State::Absent)
    );
    assert!(super::super::recovery::target(&index, before, "source.c").is_none());
    assert!(super::super::recovery::target(&index, before, "source2/deleted.txt").is_none());
}

#[test]
fn dot_explicitly_covers_the_eligible_workspace() {
    let fixture = Fixture::new();
    fixture.write("top.txt", b"top");
    fixture.write("target/ignored.bin", b"excluded");
    let workspace = workspace(&fixture);
    let saved = capture_paths(&workspace, &["."]);
    assert_eq!(saved.complete_dirs, [""]);
    let snapshot = index(&workspace);
    assert_eq!(
        super::super::recovery::target(&snapshot, saved.snapshot.unwrap(), "new.txt"),
        Some(State::Absent)
    );
    assert_eq!(
        super::super::recovery::target(&snapshot, saved.snapshot.unwrap(), "target/ignored.bin"),
        None
    );
}

#[test]
fn recursive_scope_records_new_descendants_after_command() {
    let fixture = Fixture::new();
    fixture.write("src/old.txt", b"old");
    let workspace = workspace(&fixture);
    let saved = capture_paths(&workspace, &["src"]);
    assert_eq!(saved.complete_dirs, ["src"]);
    let before = saved.snapshot.unwrap();
    fs::remove_file(fixture.path("src/old.txt")).unwrap();
    fixture.write("src/nested/new.txt", b"new");
    let after = reconcile(&workspace, &saved, "external", &Cancellation::default()).unwrap();
    assert!(after.partial.is_empty(), "{:?}", after.partial);
    let index = index(&workspace);
    assert_eq!(
        super::super::recovery::target(&index, before, "src/nested/new.txt"),
        Some(State::Absent)
    );
    assert!(matches!(index.current_state("src/old.txt"), State::Absent));
    assert!(matches!(
        index.current_state("src/nested/new.txt"),
        State::File { .. }
    ));
    assert!(index.current("outside.txt").is_none());
}

#[test]
fn absent_directory_scope_knows_future_children_were_absent() {
    let fixture = Fixture::new();
    let workspace = workspace(&fixture);
    let saved = capture_paths(&workspace, &["generated/"]);
    assert_eq!(saved.complete_dirs, ["generated"]);
    fixture.write("generated/new.txt", b"new");
    reconcile(&workspace, &saved, "external", &Cancellation::default()).unwrap();
    let index = index(&workspace);
    assert_eq!(
        super::super::recovery::target(&index, saved.snapshot.unwrap(), "generated/new.txt"),
        Some(State::Absent)
    );
}

#[test]
fn declared_directory_can_replace_an_existing_file() {
    let fixture = Fixture::new();
    fixture.write("generated", b"original file");
    let workspace = workspace(&fixture);
    let saved = capture_paths(&workspace, &["generated/"]);
    assert_eq!(saved.complete_dirs, ["generated"]);
    fs::remove_file(fixture.path("generated")).unwrap();
    fixture.write("generated/new.txt", b"new");
    reconcile(&workspace, &saved, "external", &Cancellation::default()).unwrap();
    let snapshot = index(&workspace);
    assert!(matches!(
        super::super::recovery::target(&snapshot, saved.snapshot.unwrap(), "generated"),
        Some(State::File { .. })
    ));
    assert_eq!(
        super::super::recovery::target(&snapshot, saved.snapshot.unwrap(), "generated/new.txt"),
        Some(State::Absent)
    );
}

#[test]
fn excluded_descendants_are_never_inferred_absent() {
    let fixture = Fixture::new();
    fixture.write("src/a.txt", b"a");
    fixture.write("src/target/ignored.bin", b"ignored");
    let workspace = workspace(&fixture);
    let saved = capture_paths(&workspace, &["src"]);
    assert_eq!(saved.complete_dirs, ["src"]);
    assert_eq!(
        super::super::recovery::target(
            &index(&workspace),
            saved.snapshot.unwrap(),
            "src/target/ignored.bin"
        ),
        None
    );
}

#[test]
fn cancellation_stops_inventory_without_publishing_a_complete_scope() {
    let fixture = Fixture::new();
    fixture.write("src/a.txt", b"a");
    let workspace = workspace(&fixture);
    let cancellation = Cancellation::default();
    cancellation.cancel();
    let error = capture(&workspace, &["src".into()], "test", &cancellation).unwrap_err();
    assert!(error.contains("cancelled"), "{error}");
}

#[cfg(unix)]
#[test]
fn directory_link_is_saved_as_a_link_without_following_its_target() {
    use std::os::unix::fs::symlink;
    let fixture = Fixture::new();
    let outside = crate::test_support::Directory::new();
    fs::write(outside.path().join("secret.txt"), b"secret").unwrap();
    symlink(outside.path(), fixture.path("linked")).unwrap();
    let workspace = workspace(&fixture);
    let saved = capture_paths(&workspace, &["linked", "."]);
    assert!(saved.partial.is_empty(), "{:?}", saved.partial);
    assert!(saved.selection.files.is_empty());
    assert_eq!(saved.complete_dirs, [""]);
    let index = index(&workspace);
    assert!(matches!(
        super::super::recovery::target(&index, saved.snapshot.unwrap(), "linked"),
        Some(State::Link { .. })
    ));
    assert!(index.current("linked/secret.txt").is_none());
}
