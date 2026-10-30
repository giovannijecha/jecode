use super::*;

#[test]
fn failed_copy_cleans_owned_tree_and_allows_retry() {
    let (_temp, project, storage) = fixture();
    fs::create_dir(project.join("nested")).unwrap();
    fs::write(project.join("nested/inner.txt"), "inner\n").unwrap();
    copy::FAIL.with(|fail| fail.set(true));
    let error = Workspace::prepare(&project, &storage, "a1", Mode::Isolated)
        .err()
        .unwrap();
    copy::FAIL.with(|fail| fail.set(false));
    assert!(error.contains("Injected"), "{error}");
    assert!(!storage.join("a1").exists());
    let workspace = Workspace::prepare(&project, &storage, "a1", Mode::Isolated).unwrap();
    workspace.close().unwrap();
}

fn omitted(value: &Value) -> Vec<(String, String)> {
    value
        .get("entries")
        .unwrap()
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| {
            (
                entry.get("path").unwrap().as_str().unwrap().to_owned(),
                entry.get("reason").unwrap().as_str().unwrap().to_owned(),
            )
        })
        .collect()
}

#[test]
fn entries_the_copy_cannot_hold_are_omitted_and_reported() {
    let (_temp, project, storage) = fixture();
    fs::create_dir_all(project.join("tmp/sockets")).unwrap();
    fs::write(project.join("tmp/kept.txt"), "kept\n").unwrap();
    // An unreadable file on Windows; a socket and unportable or non-UTF-8
    // names on Unix.
    #[cfg(windows)]
    let blocker = {
        use std::os::windows::fs::OpenOptionsExt;
        fs::write(project.join("tmp/locked.db"), "busy").unwrap();
        fs::OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(project.join("tmp/locked.db"))
            .unwrap()
    };
    #[cfg(windows)]
    let expected = ["tmp/locked.db"];
    #[cfg(unix)]
    let _socket =
        std::os::unix::net::UnixListener::bind(project.join("tmp/sockets/puma.sock")).unwrap();
    #[cfg(unix)]
    let expected = {
        use std::os::unix::ffi::OsStrExt;
        fs::write(project.join("tmp/a:b.txt"), "colon").unwrap();
        // APFS refuses non-UTF-8 names; only filesystems that hold one test it.
        let invalid = fs::write(
            project
                .join("tmp")
                .join(std::ffi::OsStr::from_bytes(b"bad\xffname")),
            "x",
        )
        .is_ok();
        let mut expected = vec!["tmp/a:b.txt", "tmp/sockets/puma.sock"];
        if invalid {
            expected.insert(1, "tmp/bad\u{fffd}name");
        }
        expected
    };
    let workspace = Workspace::prepare(&project, &storage, "a1", Mode::Isolated).unwrap();
    #[cfg(windows)]
    drop(blocker);
    let report = workspace.omitted().unwrap();
    let mut names: Vec<_> = omitted(&report).into_iter().map(|(name, _)| name).collect();
    names.sort();
    assert_eq!(names, expected);
    assert_eq!(
        report.get("count").unwrap().as_usize(),
        Some(expected.len())
    );
    assert_eq!(
        fs::read_to_string(workspace.root().join("tmp/kept.txt")).unwrap(),
        "kept\n"
    );
    let review = workspace.inspect().unwrap();
    assert!(changed(&review).is_empty());
    assert_eq!(review.get("omitted_from_copy"), Some(&report));
    let restored = Workspace::from_value(&workspace.value()).unwrap();
    assert_eq!(restored.omitted(), Some(report));
    restored.close().unwrap();
}

#[test]
fn copied_files_with_recorded_times_still_detect_same_size_edits() {
    let (_temp, project, storage) = fixture();
    let old = std::time::SystemTime::now() - std::time::Duration::from_secs(3600);
    for name in ["tracked.txt", "other.txt"] {
        fs::write(project.join(name), "head\n").unwrap();
        fs::File::options()
            .write(true)
            .open(project.join(name))
            .unwrap()
            .set_modified(old)
            .unwrap();
    }
    let workspace = Workspace::prepare(&project, &storage, "a1", Mode::Isolated).unwrap();
    assert_eq!(workspace.stamps.len(), 2);
    assert_eq!(
        fs::metadata(workspace.root().join("tracked.txt"))
            .unwrap()
            .modified()
            .unwrap(),
        old
    );
    assert!(changed(&workspace.inspect().unwrap()).is_empty());
    fs::write(workspace.root().join("tracked.txt"), "edit\n").unwrap();
    assert_eq!(changed(&workspace.inspect().unwrap()), vec!["tracked.txt"]);
    let restored = Workspace::from_value(&workspace.value()).unwrap();
    assert_eq!(restored.stamps, workspace.stamps);
    assert_eq!(changed(&restored.inspect().unwrap()), vec!["tracked.txt"]);
    restored.close().unwrap();
}

#[test]
fn read_only_directories_are_copied_and_removed() {
    let (_temp, project, storage) = fixture();
    let locked = project.join("locked");
    fs::create_dir(&locked).unwrap();
    fs::write(locked.join("inner.txt"), "inner\n").unwrap();
    #[cfg(unix)]
    let restrictive = 0o555;
    #[cfg(not(unix))]
    let restrictive = 1;
    files::apply_mode(&locked, restrictive).unwrap();
    let prepared = Workspace::prepare(&project, &storage, "a1", Mode::Isolated);
    let original = fs::symlink_metadata(&locked).unwrap();
    files::apply_mode(&locked, if cfg!(unix) { 0o755 } else { 0 }).unwrap();
    let workspace = prepared.unwrap();
    let copied = fs::symlink_metadata(workspace.root().join("locked")).unwrap();
    assert_eq!(files::mode(&copied), files::mode(&original));
    assert_eq!(
        fs::read_to_string(workspace.root().join("locked/inner.txt")).unwrap(),
        "inner\n"
    );
    workspace.close().unwrap();
    assert!(!storage.join("a1").exists());
}

#[test]
fn links_into_the_project_resolve_inside_the_copy() {
    let (_temp, project, storage) = fixture();
    let store = project.join("node_modules/.pnpm/pkg");
    fs::create_dir_all(&store).unwrap();
    fs::write(store.join("index.js"), "original\n").unwrap();
    link_directory(&store, &project.join("node_modules/pkg"));
    // A root `target` link is left out like a root `target` directory.
    link_directory(&store, &project.join("target"));
    let workspace = Workspace::prepare(&project, &storage, "a1", Mode::Isolated).unwrap();
    assert!(
        fs::symlink_metadata(workspace.root().join("target")).is_err(),
        "root target link must not be copied"
    );
    let link = workspace.root().join("node_modules/pkg");
    if fs::symlink_metadata(&link).is_err() {
        // Creating a symbolic link can require privileges on Windows.
        let report = omitted(&workspace.omitted().unwrap());
        assert_eq!(report.len(), 1);
        assert_eq!(report[0].0, "node_modules/pkg");
        assert!(report[0].1.contains("Could not create link"), "{report:?}");
    } else {
        assert!(
            fs::canonicalize(&link)
                .unwrap()
                .starts_with(fs::canonicalize(workspace.root()).unwrap())
        );
        fs::write(link.join("index.js"), "child\n").unwrap();
        assert_eq!(
            fs::read_to_string(store.join("index.js")).unwrap(),
            "original\n"
        );
    }
    workspace.close().unwrap();
    assert_eq!(
        fs::read_to_string(store.join("index.js")).unwrap(),
        "original\n"
    );
}

#[test]
fn snapshot_does_not_follow_project_directory_links() {
    let (temp, project, storage) = fixture();
    let outside = temp.path().join("outside");
    fs::create_dir(&outside).unwrap();
    fs::write(outside.join("canary.txt"), "keep\n").unwrap();
    link_directory(&outside, &project.join("outside-link"));
    let workspace = Workspace::prepare(&project, &storage, "a1", Mode::Isolated).unwrap();
    if let Some(report) = workspace.omitted() {
        // Creating a copied symbolic link can require privileges on Windows.
        assert_eq!(omitted(&report)[0].0, "outside-link");
        assert!(!workspace.root().join("outside-link").exists());
    } else {
        assert!(workspace.root().join("outside-link").exists());
    }
    assert!(changed(&workspace.inspect().unwrap()).is_empty());
    workspace.close().unwrap();
    assert_eq!(
        fs::read_to_string(outside.join("canary.txt")).unwrap(),
        "keep\n"
    );
}

#[cfg(unix)]
#[test]
fn sockets_created_in_the_copy_are_reported_and_removed() {
    let (_temp, project, storage) = fixture();
    let workspace = Workspace::prepare(&project, &storage, "a1", Mode::Isolated).unwrap();
    let _socket =
        std::os::unix::net::UnixListener::bind(workspace.root().join("app.sock")).unwrap();
    let review = workspace.inspect().unwrap();
    assert!(changed(&review).is_empty());
    let unsupported = review.get("unsupported_in_copy").unwrap();
    assert_eq!(omitted(unsupported)[0].0, "app.sock");
    workspace.close().unwrap();
    assert!(!storage.join("a1").exists());
}
