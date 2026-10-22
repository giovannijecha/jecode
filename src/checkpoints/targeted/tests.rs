use super::*;
use crate::checkpoints::tests::{Fixture, index, noise};
use crate::checkpoints::workspace::{self, Options};

fn offline(fixture: &Fixture) -> Arc<Workspace> {
    workspace::open(fixture.home.path(), fixture.root.path(), Options::offline()).unwrap()
}

fn call(workspace: &Arc<Workspace>, paths: &[&str]) -> Value {
    run(
        workspace,
        "model",
        &paths.iter().map(|path| (*path).into()).collect::<Vec<_>>(),
    )
    .unwrap()
}

fn item<'a>(result: &'a Value, path: &str) -> &'a Value {
    result
        .get("paths")
        .unwrap()
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item.get("path").and_then(Value::as_str) == Some(path))
        .unwrap()
}

#[test]
fn exact_preimages_include_present_and_absent_paths_without_a_baseline() {
    let fixture = Fixture::new();
    fixture.write("work.bin", b"first\0binary\r\n");
    let workspace = offline(&fixture);
    let captured = call(&workspace, &["work.bin", "created.txt"]);
    assert_eq!(
        captured.get("status").and_then(Value::as_str),
        Some("saved")
    );
    assert_eq!(captured.get("saved").and_then(Value::as_usize), Some(2));
    assert_eq!(
        item(&captured, "created.txt")
            .get("state")
            .and_then(Value::as_str),
        Some("absent")
    );
    let before = crate::checkpoints::index::parse_reference(
        captured.get("before").unwrap().as_str().unwrap(),
    )
    .unwrap();
    let snapshot = index(&workspace);
    assert!(matches!(
        recovery::target(&snapshot, before, "work.bin"),
        Some(crate::checkpoints::record::State::File { .. })
    ));
    assert_eq!(
        recovery::target(&snapshot, before, "created.txt"),
        Some(crate::checkpoints::record::State::Absent)
    );
    assert!(snapshot.current("unrelated.txt").is_none());
}

#[test]
fn recursive_capture_covers_future_children_but_not_excluded_paths() {
    let fixture = Fixture::new();
    fixture.write("src/old.txt", b"old");
    fixture.write("src/target/ignored.bin", b"ignored");
    let workspace = offline(&fixture);
    let captured = call(&workspace, &["src"]);
    assert_eq!(
        captured.get("status").and_then(Value::as_str),
        Some("saved")
    );
    assert_eq!(
        item(&captured, "src").get("status").and_then(Value::as_str),
        Some("saved")
    );
    let before = crate::checkpoints::index::parse_reference(
        captured.get("before").unwrap().as_str().unwrap(),
    )
    .unwrap();
    let snapshot = index(&workspace);
    assert_eq!(
        recovery::target(&snapshot, before, "src/nested/new.txt"),
        Some(crate::checkpoints::record::State::Absent)
    );
    assert_eq!(
        recovery::target(&snapshot, before, "src/target/ignored.bin"),
        None
    );
}

#[test]
fn whole_workspace_capture_describes_the_root_as_a_directory_scope() {
    let fixture = Fixture::new();
    fixture.write("work.txt", b"before");
    let workspace = offline(&fixture);
    let captured = call(&workspace, &["."]);
    assert_eq!(
        captured.get("status").and_then(Value::as_str),
        Some("saved")
    );
    assert_eq!(
        item(&captured, ".").get("status").and_then(Value::as_str),
        Some("saved")
    );
    assert_eq!(
        item(&captured, ".").get("state").and_then(Value::as_str),
        Some("directory scope")
    );
}

#[test]
fn absent_requested_directory_keeps_its_absent_preimage() {
    let fixture = Fixture::new();
    let workspace = offline(&fixture);
    let captured = call(&workspace, &["generated/"]);
    assert_eq!(
        captured.get("status").and_then(Value::as_str),
        Some("saved")
    );
    assert_eq!(
        item(&captured, "generated/")
            .get("state")
            .and_then(Value::as_str),
        Some("absent")
    );
}

#[test]
fn excluded_and_oversized_paths_are_reported_without_false_saves() {
    let fixture = Fixture::new();
    fixture.write("work.txt", b"small");
    fixture.write("big.bin", &noise(57, 300 << 10));
    let workspace = offline(&fixture);
    workspace
        .configure(Some(1 << 20), Some(256 << 10), "test")
        .unwrap();
    let captured = call(&workspace, &["work.txt", ".git/config", "big.bin"]);
    assert_eq!(
        captured.get("status").and_then(Value::as_str),
        Some("partial")
    );
    assert_eq!(captured.get("saved").and_then(Value::as_usize), Some(1));
    assert_eq!(captured.get("skipped").and_then(Value::as_usize), Some(1));
    assert_eq!(
        item(&captured, ".git/config")
            .get("status")
            .and_then(Value::as_str),
        Some("skipped")
    );
    assert_eq!(
        item(&captured, "big.bin")
            .get("status")
            .and_then(Value::as_str),
        Some("unprotected")
    );
}

#[test]
fn archive_capacity_failure_reports_no_saved_preimage() {
    let fixture = Fixture::new();
    fixture.write("work.txt", b"before\n");
    let workspace = offline(&fixture);
    let generation = workspace.store().journal.generation;
    let used = crate::checkpoints::quota::measure(&workspace.archive, generation).unwrap();
    workspace.store().journal.quota_limit = 2 * used.generation;
    let captured = call(&workspace, &["work.txt"]);
    assert_eq!(captured.get("status").and_then(Value::as_str), Some("none"));
    assert_eq!(captured.get("saved").and_then(Value::as_usize), Some(0));
    assert_eq!(
        captured.get("unprotected").and_then(Value::as_usize),
        Some(1)
    );
    assert!(captured.get("before").is_none());
    assert_eq!(
        item(&captured, "work.txt")
            .get("status")
            .and_then(Value::as_str),
        Some("unprotected")
    );
}
