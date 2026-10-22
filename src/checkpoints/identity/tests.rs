use super::*;
use crate::test_support::Directory;

#[test]
fn equivalent_paths_share_one_archive_and_collisions_get_a_suffix() {
    let fixture = Directory::new();
    let home = fixture.path().join("home");
    let workspace = fixture.path().join("Work");
    fs::create_dir_all(workspace.join("sub")).unwrap();
    let first = locate(&home, &workspace, true).unwrap().unwrap();
    let again = locate(&home, &workspace.join("sub").join(".."), false)
        .unwrap()
        .unwrap();
    assert_eq!(first.directory, again.directory);
    #[cfg(windows)]
    {
        let alias = fixture.path().join("WORK");
        let other = locate(&home, &alias, false).unwrap().unwrap();
        assert_eq!(first.directory, other.directory);
    }
    #[cfg(unix)]
    {
        let link = fixture.path().join("link");
        std::os::unix::fs::symlink(&workspace, &link).unwrap();
        let other = locate(&home, &link, false).unwrap().unwrap();
        assert_eq!(first.directory, other.directory);
    }
    // Simulate a different root that hashes to the same archive name.
    fs::write(
        first.directory.join("workspace.json"),
        r#"{"version":1,"root":"elsewhere"}"#,
    )
    .unwrap();
    let moved = locate(&home, &workspace, true).unwrap().unwrap();
    assert_ne!(moved.directory, first.directory);
    assert!(
        moved
            .directory
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("Work-")
    );
    fs::write(moved.directory.join("workspace.json"), b"{broken").unwrap();
    assert!(locate(&home, &workspace, true).is_err());
    assert!(locate(&home, &fixture.path().join("missing"), false).is_err());
}

#[test]
fn duplicate_legacy_and_new_roots_do_not_silently_split_history() {
    let fixture = Directory::new();
    let home = fixture.path().join("home");
    let workspace = fixture.path().join("Work");
    fs::create_dir(&workspace).unwrap();
    let new = locate(&home, &workspace, true).unwrap().unwrap();
    let hash = new
        .directory
        .file_name()
        .unwrap()
        .to_string_lossy()
        .strip_prefix("Work-")
        .unwrap()
        .to_string();
    let old = home.join("checkpoints").join(hash);
    // Reproduce archives created before the legacy-path reservation existed.
    fs::remove_file(&old).unwrap();
    fs::create_dir(&old).unwrap();
    let text = fs::read_to_string(new.directory.join("workspace.json")).unwrap();
    fs::write(
        old.join("workspace.json"),
        text.replace("\"version\": 2", "\"version\": 1"),
    )
    .unwrap();
    assert!(locate(&home, &workspace, false).is_err());
}

#[test]
fn relative_paths_resolve_aliases_without_following_the_entry() {
    let fixture = Directory::new();
    fs::create_dir_all(fixture.path().join("Work").join("src")).unwrap();
    let root = fs::canonicalize(fixture.path().join("Work")).unwrap();
    let plain_root = plain(&root);
    assert_eq!(
        relative(&root, &absolute(&plain_root, "src/new/b.txt")).as_deref(),
        Some("src/new/b.txt")
    );
    assert_eq!(relative(&root, &root), None);
    assert_eq!(relative(&root, &plain_root.join("..").join("Other")), None);
    #[cfg(windows)]
    {
        assert!(!plain_root.to_string_lossy().starts_with(r"\\?\"));
        let alias = fixture.path().join("WORK").join("SRC").join("a.rs");
        assert_eq!(relative(&root, &alias).as_deref(), Some("src/a.rs"));
        fs::write(root.join("src/Case.txt"), b"case").unwrap();
        let file_alias = fixture.path().join("WORK/SRC/cASE.TXT");
        assert_eq!(
            relative(&root, &file_alias).as_deref(),
            Some("src/Case.txt")
        );
        // The last entry itself is never followed, and a new file keeps
        // the spelling supplied by its creator.
        assert_eq!(
            relative(&root, &fixture.path().join("WORK/SRC/New.txt")).as_deref(),
            Some("src/New.txt")
        );
    }
    #[cfg(unix)]
    {
        let link = fixture.path().join("link");
        std::os::unix::fs::symlink(&root, &link).unwrap();
        assert_eq!(
            relative(&root, &link.join("src/a.rs")).as_deref(),
            Some("src/a.rs")
        );
        // A link inside the workspace is the entry itself.
        std::os::unix::fs::symlink("/elsewhere", root.join("src/out")).unwrap();
        assert_eq!(
            relative(&root, &root.join("src/out")).as_deref(),
            Some("src/out")
        );
    }
}
