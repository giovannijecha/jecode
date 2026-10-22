use super::*;
use crate::cancel::Cancellation;
use crate::checkpoints::scopes;

#[cfg(any(windows, target_os = "macos"))]
#[test]
fn group_restore_merges_case_only_renames_in_both_directions() {
    for (first, second) in [("Case.txt", "case.txt"), ("case.txt", "Case.txt")] {
        let fixture = Fixture::new();
        fixture.write(first, b"first\n");
        if !fixture.path(second).exists() {
            return;
        }
        let workspace =
            workspace::open(fixture.home.path(), fixture.root.path(), Options::offline()).unwrap();
        let initial = scopes::capture(
            &workspace,
            &paths(&["."]),
            "model",
            &Cancellation::default(),
        )
        .unwrap();
        fs::rename(fixture.path(first), fixture.path("intermediate.txt")).unwrap();
        fs::rename(fixture.path("intermediate.txt"), fixture.path(second)).unwrap();
        fixture.write(second, b"second\n");
        let later = scopes::capture(
            &workspace,
            &paths(&["."]),
            "model",
            &Cancellation::default(),
        )
        .unwrap();
        assert_eq!(later.group, initial.group);
        fixture.write(second, b"third\n");

        let report = workspace
            .restore(&request(
                &workspace,
                initial.group.unwrap(),
                None,
                Expected::Latest,
            ))
            .unwrap();
        assert!(report.failed.is_none() && report.conflicts.is_empty());
        assert_eq!(fixture.read(first).unwrap(), b"first\n");
        assert_eq!(fixture.read(second).unwrap(), b"first\n");
        assert_eq!(report.applied.len(), 1, "{} -> {}", first, second);
        assert_eq!(fs::read_dir(fixture.root.path()).unwrap().count(), 1);
    }
}

#[cfg(any(windows, target_os = "macos"))]
#[test]
fn group_restore_merges_case_only_directory_renames() {
    for (first, second) in [("Dir", "dir"), ("dir", "Dir")] {
        let fixture = Fixture::new();
        fixture.write(&format!("{first}/file.txt"), b"first\n");
        if !fixture.path(&format!("{second}/file.txt")).exists() {
            return;
        }
        let workspace =
            workspace::open(fixture.home.path(), fixture.root.path(), Options::offline()).unwrap();
        let initial = scopes::capture(
            &workspace,
            &paths(&["."]),
            "model",
            &Cancellation::default(),
        )
        .unwrap();
        fs::rename(fixture.path(first), fixture.path("intermediate")).unwrap();
        fs::rename(fixture.path("intermediate"), fixture.path(second)).unwrap();
        fixture.write(&format!("{second}/file.txt"), b"second\n");
        let later = scopes::capture(
            &workspace,
            &paths(&["."]),
            "model",
            &Cancellation::default(),
        )
        .unwrap();
        assert_eq!(later.group, initial.group);
        fixture.write(&format!("{second}/file.txt"), b"third\n");

        let report = workspace
            .restore(&request(
                &workspace,
                initial.group.unwrap(),
                None,
                Expected::Latest,
            ))
            .unwrap();
        assert!(report.failed.is_none() && report.conflicts.is_empty());
        assert_eq!(
            fixture.read(&format!("{first}/file.txt")).unwrap(),
            b"first\n"
        );
        assert_eq!(fs::read_dir(fixture.root.path()).unwrap().count(), 1);
        assert_eq!(fs::read_dir(fixture.path(first)).unwrap().count(), 1);
    }
}

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

#[test]
fn full_restore_ignores_unrelated_deleted_unicode_history() {
    for scope in ["src/main.rs", "src/"] {
        let fixture = Fixture::new();
        fixture.write("docs/perché.md", b"old document\n");
        fixture.write("src/main.rs", b"original\n");
        let workspace =
            workspace::open(fixture.home.path(), fixture.root.path(), Options::offline()).unwrap();
        let old = scopes::capture(
            &workspace,
            &paths(&["docs/perché.md"]),
            "earlier",
            &Cancellation::default(),
        )
        .unwrap();
        fs::remove_file(fixture.path("docs/perché.md")).unwrap();
        scopes::reconcile(&workspace, &old, "external", &Cancellation::default()).unwrap();
        let current = scopes::capture(
            &workspace,
            &paths(&[scope]),
            "model",
            &Cancellation::default(),
        )
        .unwrap();
        fixture.write("src/main.rs", b"changed\n");

        let report = workspace
            .restore(&request(
                &workspace,
                current.group.unwrap(),
                None,
                Expected::Latest,
            ))
            .unwrap();
        assert!(report.failed.is_none() && report.conflicts.is_empty());
        assert_eq!(restored(&report), paths(&["src/main.rs"]), "{scope}");
        assert_eq!(fixture.read("src/main.rs").unwrap(), b"original\n");
        assert!(fixture.read("docs/perché.md").is_none());
    }
}

#[test]
fn wide_unicode_catalog_is_probed_once_per_index_kind() {
    let fixture = Fixture::new();
    for number in 0..80 {
        fixture.write(&format!("plain-{number}.txt"), b"saved\n");
        fixture.write(&format!("accent-{number}-\u{e9}.txt"), b"saved\n");
    }
    let workspace =
        workspace::open(fixture.home.path(), fixture.root.path(), Options::offline()).unwrap();
    let captured = scopes::capture(
        &workspace,
        &paths(&["."]),
        "model",
        &Cancellation::default(),
    )
    .unwrap();
    let names = crate::checkpoints::restore::TargetNames::new(
        fixture.root.path(),
        &index(&workspace),
        captured.group.unwrap(),
    );
    assert_eq!(names.index_probes(), 0);
    assert_eq!(names.resolve("plain-0.txt").unwrap(), "plain-0.txt");
    assert_eq!(names.index_probes(), 80);
    for number in 0..80 {
        let unicode = format!("accent-{number}-\u{e9}.txt");
        let ascii = format!("plain-{number}.txt");
        assert_eq!(names.resolve(&unicode).unwrap(), unicode);
        assert_eq!(names.resolve(&ascii).unwrap(), ascii);
    }
    assert_eq!(names.index_probes(), 240);
}

#[cfg(target_os = "macos")]
#[test]
fn unicode_alias_with_a_second_hard_link_cannot_be_deleted_as_absent() {
    const SAVED: &str = "caf\u{e9}.txt";
    const RENAMED: &str = "cafe\u{301}.txt";
    let fixture = Fixture::new();
    fixture.write(SAVED, b"original\n");
    fs::hard_link(fixture.path(SAVED), fixture.path("other.txt")).unwrap();
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
    if !fixture.path(SAVED).exists() {
        return; // This filesystem treats the two normalization forms as distinct.
    }

    match workspace.restore(&request(&workspace, target, None, Expected::Latest)) {
        Ok(report) => {
            assert!(report.failed.is_none() && report.conflicts.is_empty());
            assert_eq!(fixture.read(SAVED).unwrap(), b"original\n");
            assert_eq!(fixture.read(RENAMED).unwrap(), b"original\n");
            assert_eq!(fixture.read("other.txt").unwrap(), b"original\n");
            assert_eq!(fs::read_dir(fixture.root.path()).unwrap().count(), 2);
        }
        Err(_) => {
            assert_eq!(fixture.read(SAVED).unwrap(), b"changed\n");
            assert_eq!(fixture.read(RENAMED).unwrap(), b"changed\n");
            assert_eq!(fixture.read("other.txt").unwrap(), b"changed\n");
            assert_eq!(fs::read_dir(fixture.root.path()).unwrap().count(), 2);
        }
    }
}

#[cfg(target_os = "macos")]
#[test]
fn exact_distinct_hard_links_keep_their_own_recorded_spelling() {
    let fixture = Fixture::new();
    fixture.write("first.txt", b"saved\n");
    fs::hard_link(fixture.path("first.txt"), fixture.path("second.txt")).unwrap();
    let workspace =
        workspace::open(fixture.home.path(), fixture.root.path(), Options::offline()).unwrap();
    let captured = scopes::capture(
        &workspace,
        &paths(&["."]),
        "model",
        &Cancellation::default(),
    )
    .unwrap();
    let names = crate::checkpoints::restore::TargetNames::new(
        fixture.root.path(),
        &index(&workspace),
        captured.group.unwrap(),
    );
    assert_eq!(names.resolve("first.txt").unwrap(), "first.txt");
    assert_eq!(names.resolve("second.txt").unwrap(), "second.txt");
}
