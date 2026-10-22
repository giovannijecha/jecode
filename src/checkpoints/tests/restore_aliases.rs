use super::*;
use crate::cancel::Cancellation;
use crate::checkpoints::scopes;

fn saved_case(fixture: &Fixture) -> (Arc<Workspace>, u64) {
    fixture.write("Case.txt", b"original\n");
    let workspace =
        workspace::open(fixture.home.path(), fixture.root.path(), Options::offline()).unwrap();
    let captured = scopes::capture(
        &workspace,
        &paths(&["."]),
        "model",
        &Cancellation::default(),
    )
    .unwrap();
    (workspace, captured.group.unwrap())
}

#[test]
fn existing_case_variant_resolves_to_the_saved_file() {
    let fixture = Fixture::new();
    let (workspace, group) = saved_case(&fixture);
    fixture.write("Case.txt", b"changed\n");
    let selected = paths(&["case.txt"]);

    let report = workspace
        .restore(&request(
            &workspace,
            group,
            Some(&selected),
            Expected::Latest,
        ))
        .unwrap();
    assert!(report.failed.is_none() && report.conflicts.is_empty());
    assert_eq!(fixture.read("Case.txt").unwrap(), b"original\n");
    assert_eq!(restored(&report), paths(&["Case.txt"]));
}

#[test]
fn existing_case_variant_resolves_to_a_legacy_point() {
    let fixture = Fixture::new();
    fixture.write("Case.txt", b"original\n");
    let workspace = fixture.open();
    let target = index(&workspace).last_point().unwrap().seq;
    fixture.write("Case.txt", b"changed\n");
    let selected = paths(&["case.txt"]);

    let report = workspace
        .restore(&request(
            &workspace,
            target,
            Some(&selected),
            Expected::Latest,
        ))
        .unwrap();
    assert!(report.failed.is_none() && report.conflicts.is_empty());
    assert_eq!(fixture.read("Case.txt").unwrap(), b"original\n");
    assert_eq!(restored(&report), paths(&["Case.txt"]));
}

#[test]
fn deleted_case_variant_requires_the_recorded_spelling() {
    let fixture = Fixture::new();
    let (workspace, group) = saved_case(&fixture);
    fs::remove_file(fixture.path("Case.txt")).unwrap();
    let selected = paths(&["case.txt"]);

    let error = workspace
        .restore(&request(
            &workspace,
            group,
            Some(&selected),
            Expected::Latest,
        ))
        .err()
        .expect("missing case alias needs proof");
    assert!(error.contains("Case.txt"), "{error}");
    assert!(fixture.read("Case.txt").is_none());
    assert!(fixture.read("case.txt").is_none());
    let exact = paths(&["Case.txt"]);
    let report = workspace
        .restore(&request(&workspace, group, Some(&exact), Expected::Latest))
        .unwrap();
    assert!(report.failed.is_none() && report.conflicts.is_empty());
    assert_eq!(fixture.read("Case.txt").unwrap(), b"original\n");
}

#[test]
fn deleted_nested_case_variant_cannot_infer_absence_from_its_parent_scope() {
    let fixture = Fixture::new();
    fixture.write("Dir/Case.txt", b"original\n");
    let workspace =
        workspace::open(fixture.home.path(), fixture.root.path(), Options::offline()).unwrap();
    let captured = scopes::capture(
        &workspace,
        &paths(&["Dir/"]),
        "model",
        &Cancellation::default(),
    )
    .unwrap();
    let group = captured.group.unwrap();
    fs::remove_file(fixture.path("Dir/Case.txt")).unwrap();
    let selected = paths(&["dir/case.txt"]);

    let error = workspace
        .restore(&request(
            &workspace,
            group,
            Some(&selected),
            Expected::Latest,
        ))
        .err()
        .expect("missing case alias needs proof");
    assert!(error.contains("Dir/Case.txt"), "{error}");
    assert!(fixture.read("Dir/Case.txt").is_none());
}

#[test]
fn full_restore_deduplicates_a_case_only_rename() {
    let fixture = Fixture::new();
    let (workspace, group) = saved_case(&fixture);
    fs::rename(fixture.path("Case.txt"), fixture.path("intermediate.txt")).unwrap();
    fs::rename(fixture.path("intermediate.txt"), fixture.path("case.txt")).unwrap();
    fixture.write("case.txt", b"changed\n");

    let report = workspace
        .restore(&request(&workspace, group, None, Expected::Latest))
        .unwrap();
    assert!(report.failed.is_none() && report.conflicts.is_empty());
    assert_eq!(fixture.read("Case.txt").unwrap(), b"original\n");
    let file_actions: Vec<_> = report
        .applied
        .iter()
        .filter(|(path, _)| path.eq_ignore_ascii_case("Case.txt"))
        .collect();
    assert_eq!(file_actions.len(), 1);
    assert_eq!(file_actions[0].0, "Case.txt");
    let names: Vec<_> = fs::read_dir(fixture.root.path())
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names.len(), 1);
}
