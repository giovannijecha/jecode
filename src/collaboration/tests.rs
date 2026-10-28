use super::*;
use crate::test_support::Directory;

#[test]
fn visible_updates_and_recovered_snapshot_are_bounded() {
    let project = Directory::new();
    let storage = Directory::new();
    let workspace = Workspace::prepare(
        project.path(),
        storage.path(),
        "a1",
        workspace::Mode::Shared,
    )
    .unwrap();
    let seed = Value::object([
        ("id", Value::string("a1")),
        ("number", Value::number(1)),
        ("state", Value::string("returned")),
        ("workspace", Value::string("shared")),
        ("workspace_info", workspace.value()),
        ("directory", Value::string(project.path().to_string_lossy())),
    ]);
    let observer = Observer::default();
    observer
        .0
        .state
        .lock()
        .unwrap()
        .records
        .push(Record::from_value(&seed).unwrap());
    for _ in 0..(MAX_UPDATES + 10) {
        observer.message("a1", &"x".repeat(UPDATE_CHARS + 10));
    }
    let snapshot = &observer.snapshot()[0];
    let updates = snapshot.get("updates").and_then(Value::as_array).unwrap();
    assert_eq!(updates.len(), MAX_UPDATES);
    assert!(
        updates
            .iter()
            .all(|update| update.as_str().unwrap().chars().count() == UPDATE_CHARS)
    );
    assert_eq!(snapshot.get("updates_truncated"), Some(&Value::Bool(true)));
    let recovered = Record::from_value(snapshot).unwrap().value();
    assert_eq!(
        recovered
            .get("updates")
            .and_then(Value::as_array)
            .unwrap()
            .len(),
        MAX_UPDATES
    );
}
