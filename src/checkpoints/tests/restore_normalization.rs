use super::*;
use crate::cancel::Cancellation;
use crate::checkpoints::scopes;

#[test]
fn full_restore_handles_unicode_normalization_aliases() {
    const SAVED: &str = "caf\u{e9}.txt";
    const RENAMED: &str = "cafe\u{301}.txt";
    let fixture = Fixture::new();
    fixture.write(SAVED, b"original\n");
    let workspace =
        workspace::open(fixture.home.path(), fixture.root.path(), Options::offline()).unwrap();
    let captured = scopes::capture(
        &workspace,
        &paths(&["."]),
        "model",
        &Cancellation::default(),
    )
    .unwrap();
    let target = captured.group.unwrap();
    fs::rename(fixture.path(SAVED), fixture.path("intermediate.txt")).unwrap();
    fs::rename(fixture.path("intermediate.txt"), fixture.path(RENAMED)).unwrap();
    fixture.write(RENAMED, b"changed\n");
    let alias = fixture.path(SAVED).exists();

    let report = workspace
        .restore(&request(&workspace, target, None, Expected::Latest))
        .unwrap();
    assert!(report.failed.is_none() && report.conflicts.is_empty());
    assert_eq!(fixture.read(SAVED).unwrap(), b"original\n");
    if alias {
        assert_eq!(
            report
                .applied
                .iter()
                .filter(|(path, _)| path == SAVED || path == RENAMED)
                .count(),
            1
        );
        assert_eq!(fs::read_dir(fixture.root.path()).unwrap().count(), 1);
    } else {
        assert!(fixture.read(RENAMED).is_none());
    }
}

#[test]
fn unrelated_unicode_name_does_not_make_a_deleted_ascii_path_ambiguous() {
    let fixture = Fixture::new();
    fixture.write("caf\u{e9}.txt", b"saved\n");
    let workspace =
        workspace::open(fixture.home.path(), fixture.root.path(), Options::offline()).unwrap();
    let captured = scopes::capture(
        &workspace,
        &paths(&["."]),
        "model",
        &Cancellation::default(),
    )
    .unwrap();
    let target = captured.group.unwrap();
    fixture.write("created.txt", b"temporary\n");
    scopes::reconcile(&workspace, &captured, "model", &Cancellation::default()).unwrap();
    fs::remove_file(fixture.path("created.txt")).unwrap();
    fixture.write("caf\u{e9}.txt", b"changed\n");

    let report = workspace
        .restore(&request(&workspace, target, None, Expected::Latest))
        .unwrap();
    assert!(report.failed.is_none() && report.conflicts.is_empty());
    assert_eq!(fixture.read("caf\u{e9}.txt").unwrap(), b"saved\n");
    assert!(fixture.read("created.txt").is_none());
}
