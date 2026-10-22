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
