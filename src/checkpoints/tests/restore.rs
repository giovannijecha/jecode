use super::*;
use crate::checkpoints::record::State;
use crate::checkpoints::restore::Action;

fn baseline(workspace: &Workspace) -> u64 {
    index(workspace)
        .points
        .values()
        .find(|point| point.kind == "baseline")
        .unwrap()
        .seq
}

#[cfg(unix)]
#[test]
fn directory_modes_allow_child_restores_before_final_restrictions() {
    use std::os::unix::fs::PermissionsExt;
    for (target_mode, current_mode) in [(0o500, 0o700), (0o700, 0o500)] {
        let fixture = Fixture::new();
        fixture.write("directory/file.txt", b"original\n");
        let workspace = fixture.open();
        let path = fixture.path("directory");
        fs::set_permissions(&path, fs::Permissions::from_mode(target_mode)).unwrap();
        let target = workspace
            .mark("milestone", "read-only directory", "test", false, false)
            .unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(current_mode)).unwrap();
        fixture.write("directory/file.txt", b"edited\n");
        let report = workspace
            .restore(&request(&workspace, target, None, Expected::Latest))
            .unwrap();
        let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o7777;
        let bytes = fixture.read("directory/file.txt").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        assert!(report.failed.is_none() && report.conflicts.is_empty());
        assert_eq!(bytes, b"original\n");
        assert_eq!(mode, target_mode);
    }
}

#[test]
fn full_restore_reproduces_exact_bytes_and_permissions_and_can_be_undone() {
    let fixture = Fixture::new();
    let binary: Vec<u8> = (0..=255u8).cycle().take(70_000).collect();
    fixture.write("src/main.rs", b"fn main() {}\n");
    fixture.write("data/blob.bin", &binary);
    fixture.write("data/empty", b"");
    fixture.write(".env", b"TOKEN=synthetic\n");
    fixture.write("JECODE.md", b"Use tabs.\n");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(
            fixture.path("data/blob.bin"),
            fs::Permissions::from_mode(0o640),
        )
        .unwrap();
        fs::set_permissions(fixture.path("src"), fs::Permissions::from_mode(0o750)).unwrap();
    }
    set_readonly(&fixture.path(".env"), true);
    let workspace = fixture.open();
    let start = baseline(&workspace);

    fixture.write("src/main.rs", b"fn main() { changed() }\n");
    fs::remove_file(fixture.path("data/empty")).unwrap();
    fixture.write("src/added.rs", b"pub fn added() {}\n");
    fixture.write("extra/deep/file.txt", b"later\n");
    set_readonly(&fixture.path(".env"), false);
    fixture.write("JECODE.md", b"Use spaces.\n");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(
            fixture.path("data/blob.bin"),
            fs::Permissions::from_mode(0o600),
        )
        .unwrap();
    }
    let changed = workspace
        .mark("milestone", "changed", "test", false, false)
        .unwrap();

    let report = workspace
        .restore(&request(&workspace, start, None, Expected::Latest))
        .unwrap();
    assert!(report.conflicts.is_empty() && report.failed.is_none());
    assert!(report.instructions_changed());
    assert_eq!(fixture.read("src/main.rs").unwrap(), b"fn main() {}\n");
    assert_eq!(fixture.read("data/blob.bin").unwrap(), binary);
    assert_eq!(fixture.read("data/empty").unwrap(), b"");
    assert_eq!(fixture.read("JECODE.md").unwrap(), b"Use tabs.\n");
    assert!(fixture.read("src/added.rs").is_none());
    assert!(!fixture.path("extra").exists());
    assert!(
        fs::metadata(fixture.path(".env"))
            .unwrap()
            .permissions()
            .readonly()
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = |path: &str| {
            fs::metadata(fixture.path(path))
                .unwrap()
                .permissions()
                .mode()
                & 0o7777
        };
        assert_eq!(mode("data/blob.bin"), 0o640);
        assert_eq!(mode("src"), 0o750);
    }
    let deleted: Vec<&str> = report
        .applied
        .iter()
        .filter(|(_, action)| *action == Action::Delete)
        .map(|(path, _)| path.as_str())
        .collect();
    // Entries are removed before their directories.
    let mut sorted = deleted.clone();
    sorted.sort_unstable();
    assert_eq!(
        sorted,
        ["extra", "extra/deep", "extra/deep/file.txt", "src/added.rs"]
    );
    for (position, path) in deleted.iter().enumerate() {
        assert!(
            deleted[position..]
                .iter()
                .all(|later| !later.starts_with(&format!("{path}/"))),
            "{deleted:?}"
        );
    }

    // The state before the restore was saved and restores like any other.
    let before = report.before.unwrap();
    assert!(before > changed);
    let undo = workspace
        .restore(&request(&workspace, before, None, Expected::Latest))
        .unwrap();
    assert!(undo.failed.is_none());
    assert_eq!(
        fixture.read("src/main.rs").unwrap(),
        b"fn main() { changed() }\n"
    );
    assert_eq!(fixture.read("extra/deep/file.txt").unwrap(), b"later\n");
    assert!(fixture.read("data/empty").is_none());
    assert!(
        !fs::metadata(fixture.path(".env"))
            .unwrap()
            .permissions()
            .readonly()
    );
}

#[test]
fn selective_restore_reports_conflicts_with_a_retry_precondition() {
    let fixture = Fixture::new();
    fixture.write("notes.txt", b"one\n");
    fixture.write("other.txt", b"stay\n");
    let workspace = fixture.open();
    let start = baseline(&workspace);
    fixture.write("notes.txt", b"two\n");
    let second = workspace
        .mark("milestone", "two", "test", false, false)
        .unwrap();
    // Changed again by someone else after the requester last looked.
    fixture.write("notes.txt", b"three\n");
    fixture.write("other.txt", b"moved\n");

    let selected = paths(&["notes.txt"]);
    let report = workspace
        .restore(&request(
            &workspace,
            start,
            Some(&selected),
            Expected::At(second),
        ))
        .unwrap();
    assert!(report.applied.is_empty());
    let conflict = &report.conflicts[0];
    assert_eq!(
        (conflict.path.as_str(), conflict.target, conflict.expected),
        ("notes.txt", start, second)
    );
    assert!(conflict.current > second);
    assert_eq!(fixture.read("notes.txt").unwrap(), b"three\n");

    // Retrying with the reported current state as the precondition succeeds.
    let report = workspace
        .restore(&request(
            &workspace,
            start,
            Some(&selected),
            Expected::At(conflict.current),
        ))
        .unwrap();
    assert_eq!(restored(&report), ["notes.txt"]);
    assert_eq!(fixture.read("notes.txt").unwrap(), b"one\n");
    assert_eq!(fixture.read("other.txt").unwrap(), b"moved\n");

    // A full restore applies nothing while any path conflicts.
    fixture.write("notes.txt", b"four\n");
    let report = workspace
        .restore(&request(&workspace, second, None, Expected::At(second)))
        .unwrap();
    assert!(report.applied.is_empty() && !report.conflicts.is_empty());
    assert_eq!(fixture.read("other.txt").unwrap(), b"moved\n");

    // A preview plans without changing anything.
    let mut preview = request(&workspace, start, None, Expected::Latest);
    preview.preview = true;
    let report = workspace.restore(&preview).unwrap();
    assert!(report.preview && !report.planned.is_empty() && report.applied.is_empty());
    assert_eq!(fixture.read("notes.txt").unwrap(), b"four\n");
}

#[test]
fn scope_and_limits_classify_paths_without_reading_ignore_files() {
    let fixture = Fixture::new();
    fixture.write(".gitignore", b".env\nsecret/\n");
    fixture.write(".env", b"KEY=synthetic\n");
    fixture.write("secret/JECODE.md", b"scoped\n");
    fixture.write("node_modules/pkg/index.js", b"generated\n");
    fixture.write("target/debug/app", b"binary\n");
    fixture.write("large.bin", &noise(1, 4096));
    let workspace = fixture.open();
    let policy = workspace.configure(None, Some(1024), "test").unwrap();
    assert_eq!(policy.max_file_bytes, 1024);
    // Captured content stays; the next change over the limit is not captured.
    fixture.write("large.bin", &noise(5, 4096));
    workspace
        .mark("milestone", "limits", "test", false, false)
        .unwrap();

    let index = index(&workspace);
    assert!(
        index
            .current(".env")
            .is_some_and(|version| version.state.exists())
    );
    assert!(index.current("secret/JECODE.md").is_some());
    assert!(index.current("node_modules/pkg/index.js").is_none());
    assert!(index.current("target/debug/app").is_none());
    assert_eq!(
        index.current_state("large.bin"),
        State::Oversized { length: 4096 }
    );

    // Oversized content cannot be restored and says so.
    let before = index.latest();
    fixture.write("large.bin", &noise(2, 4096));
    let selected = paths(&["large.bin"]);
    let report = workspace
        .restore(&request(
            &workspace,
            before,
            Some(&selected),
            Expected::Latest,
        ))
        .unwrap();
    assert!(report.applied.is_empty());
    assert!(
        report.skipped[0].1.contains("not captured"),
        "{:?}",
        report.skipped
    );

    // Limits persist for the next process.
    drop(workspace);
    let reopened = fixture.open();
    assert_eq!(reopened.store().policy.max_file_bytes, 1024);
    assert!(reopened.configure(Some(512), None, "test").is_err());

    // Raising the limit captures the file without another change.
    reopened.configure(None, Some(8192), "test").unwrap();
    reopened.verify().unwrap();
    reopened
        .mark("milestone", "raised", "test", false, false)
        .unwrap();
    assert!(matches!(
        super::index(&reopened).current_state("large.bin"),
        State::File { length: 4096, .. }
    ));
}

#[test]
fn links_are_recorded_and_restored_without_being_followed() {
    let fixture = Fixture::new();
    fixture.write("target.txt", b"real\n");
    let outside = Directory::new();
    fs::write(outside.path().join("private.txt"), b"outside\n").unwrap();
    let link = fixture.path("link");
    if crate::checkpoints::files::symlink("target.txt", &link).is_err() {
        // Creating links needs a privilege some Windows accounts lack.
        return;
    }
    let workspace = fixture.open();
    let start = baseline(&workspace);
    assert_eq!(
        index(&workspace).current_state("link"),
        State::Link {
            target: "target.txt".into()
        }
    );
    crate::checkpoints::files::remove(&link).unwrap();
    fixture.write("link", b"now a file\n");
    let selected = paths(&["link"]);
    let report = workspace
        .restore(&request(
            &workspace,
            start,
            Some(&selected),
            Expected::Latest,
        ))
        .unwrap();
    assert_eq!(restored(&report), ["link"]);
    let metadata = fs::symlink_metadata(&link).unwrap();
    assert!(metadata.file_type().is_symlink());
    assert_eq!(
        fs::read_link(&link).unwrap(),
        std::path::Path::new("target.txt")
    );
}

#[test]
fn released_and_unknown_references_are_rejected() {
    let fixture = Fixture::new();
    fixture.write("a.txt", b"a\n");
    let workspace = fixture.open();
    let future = index(&workspace).latest() + 100;
    let error = workspace
        .restore(&request(&workspace, future, None, Expected::Latest))
        .err()
        .unwrap();
    assert!(error.contains(&format!("cp-{future}")), "{error}");
}

#[test]
fn restore_keeps_an_oversized_current_file_when_undo_cannot_capture_it() {
    let fixture = Fixture::new();
    fixture.write("a.txt", b"original\n");
    let workspace = fixture.open();
    let start = baseline(&workspace);
    workspace.configure(None, Some(1024), "test").unwrap();
    let current = noise(37, 2048);
    fixture.write("a.txt", &current);
    let selected = paths(&["a.txt"]);

    let report = workspace
        .restore(&request(
            &workspace,
            start,
            Some(&selected),
            Expected::Latest,
        ))
        .unwrap();
    assert!(report.applied.is_empty());
    assert!(report.before.is_none());
    assert!(
        report
            .skipped
            .iter()
            .any(|(path, reason)| { path == "a.txt" && reason.contains("not captured") })
    );
    assert_eq!(fixture.read("a.txt").unwrap(), current);
}

#[test]
fn selected_restore_does_not_follow_a_linked_parent_outside_the_workspace() {
    let fixture = Fixture::new();
    fixture.write("dir/file.txt", b"archived\n");
    let workspace = fixture.open();
    let start = baseline(&workspace);
    let outside = Directory::new();
    let outside_file = outside.path().join("file.txt");
    fs::write(&outside_file, b"outside\n").unwrap();
    fs::remove_file(fixture.path("dir/file.txt")).unwrap();
    fs::remove_dir(fixture.path("dir")).unwrap();
    let link_result =
        crate::checkpoints::files::symlink(&outside.path().to_string_lossy(), &fixture.path("dir"));
    #[cfg(windows)]
    let junction_result = if link_result.is_err() {
        use std::os::windows::process::CommandExt;
        Some(
            std::process::Command::new("cmd")
                .arg("/C")
                .raw_arg(format!(
                    "mklink /J \"{}\" \"{}\"",
                    crate::checkpoints::identity::plain(&fixture.path("dir")).display(),
                    crate::checkpoints::identity::plain(outside.path()).display()
                ))
                .output(),
        )
    } else {
        None
    };
    #[cfg(windows)]
    assert!(
        link_result.is_ok()
            || junction_result
                .as_ref()
                .is_some_and(|result| result.as_ref().is_ok_and(|output| output.status.success())),
        "could not create a directory link or junction: symlink {link_result:?}, junction {junction_result:?}"
    );
    #[cfg(not(windows))]
    assert!(
        link_result.is_ok(),
        "could not create a directory link: {link_result:?}"
    );
    let selected = paths(&["dir/file.txt"]);

    let error = workspace
        .restore(&request(
            &workspace,
            start,
            Some(&selected),
            Expected::Latest,
        ))
        .err()
        .unwrap();
    assert!(error.contains("dir/file.txt"), "{error}");
    assert_eq!(fs::read(&outside_file).unwrap(), b"outside\n");
}

#[test]
fn full_restore_replaces_a_nonempty_directory_with_its_older_file() {
    let fixture = Fixture::new();
    fixture.write("node", b"older file\n");
    let workspace = fixture.open();
    let start = baseline(&workspace);
    fs::remove_file(fixture.path("node")).unwrap();
    fixture.write("node/child", b"new child\n");
    workspace
        .mark("milestone", "directory", "test", false, false)
        .unwrap();

    let report = workspace
        .restore(&request(&workspace, start, None, Expected::Latest))
        .unwrap();
    assert!(report.failed.is_none(), "{:?}", report.failed);
    assert_eq!(fixture.read("node").unwrap(), b"older file\n");
    assert!(fixture.read("node/child").is_none());

    let undo = workspace
        .restore(&request(
            &workspace,
            report.before.unwrap(),
            None,
            Expected::Latest,
        ))
        .unwrap();
    assert!(undo.failed.is_none(), "{:?}", undo.failed);
    assert_eq!(fixture.read("node/child").unwrap(), b"new child\n");
}

#[test]
fn selected_child_restore_does_not_create_an_unrecorded_parent() {
    let fixture = Fixture::new();
    fixture.write("dir/child", b"archived\n");
    let workspace = fixture.open();
    let start = baseline(&workspace);
    fs::remove_file(fixture.path("dir/child")).unwrap();
    fs::remove_dir(fixture.path("dir")).unwrap();
    workspace
        .mark("milestone", "removed", "test", false, false)
        .unwrap();

    let selected = paths(&["dir/child"]);
    let report = workspace
        .restore(&request(
            &workspace,
            start,
            Some(&selected),
            Expected::Latest,
        ))
        .unwrap();
    assert!(report.applied.is_empty());
    assert!(report.failed.is_some());
    assert!(!fixture.path("dir").exists());
}

#[test]
fn selected_child_restore_keeps_a_file_parent_untouched() {
    let fixture = Fixture::new();
    fixture.write("dir/child", b"archived\n");
    let workspace = fixture.open();
    let start = baseline(&workspace);
    fs::remove_file(fixture.path("dir/child")).unwrap();
    fs::remove_dir(fixture.path("dir")).unwrap();
    fixture.write("dir", b"current parent file\n");
    let selected = paths(&["dir/child"]);

    let report = workspace
        .restore(&request(
            &workspace,
            start,
            Some(&selected),
            Expected::Latest,
        ))
        .unwrap();
    assert!(report.applied.is_empty());
    assert!(report.failed.is_some());
    assert_eq!(fixture.read("dir").unwrap(), b"current parent file\n");
}

#[test]
fn failed_parent_replacement_keeps_excluded_children_and_can_undo_partial_progress() {
    let fixture = Fixture::new();
    fixture.write("node", b"older file\n");
    let workspace = fixture.open();
    let start = baseline(&workspace);
    fs::remove_file(fixture.path("node")).unwrap();
    fixture.write("node/child", b"captured child\n");
    fixture.write("node/.git/keep", b"excluded\n");
    workspace
        .mark("milestone", "directory", "test", false, false)
        .unwrap();

    let report = workspace
        .restore(&request(&workspace, start, None, Expected::Latest))
        .unwrap();
    assert_eq!(restored(&report), ["node/child"]);
    assert_eq!(
        report.failed.as_ref().map(|(path, _)| path.as_str()),
        Some("node")
    );
    assert_eq!(fixture.read("node/.git/keep").unwrap(), b"excluded\n");
    assert!(fixture.read("node/child").is_none());

    let undo = workspace
        .restore(&request(
            &workspace,
            report.before.unwrap(),
            None,
            Expected::Latest,
        ))
        .unwrap();
    assert!(undo.failed.is_none(), "{:?}", undo.failed);
    assert_eq!(fixture.read("node/child").unwrap(), b"captured child\n");
    assert_eq!(fixture.read("node/.git/keep").unwrap(), b"excluded\n");
}
#[cfg(windows)]
#[test]
fn restore_rejects_embedded_drive_prefixes_before_observing_or_changing_files() {
    let fixture = Fixture::new();
    fixture.write("a/kept.txt", b"kept\n");
    let workspace = fixture.open();
    let target = workspace
        .mark("test", "before", "test", false, false)
        .unwrap();
    for path in ["a/C:outside.txt", "a/C:/outside.txt", "a/kept.txt:stream"] {
        let selected = paths(&[path]);
        let before = index(&workspace).latest();
        let result = workspace.restore(&request(
            &workspace,
            target,
            Some(&selected),
            Expected::Latest,
        ));
        assert!(result.is_err(), "invalid path was accepted: {path}");
        assert_eq!(index(&workspace).latest(), before);
        assert_eq!(fixture.read("a/kept.txt").unwrap(), b"kept\n");
    }
}
