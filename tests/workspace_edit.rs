#[path = "support/workspace.rs"]
mod support;
use jecode::workspace::{Budget, RecoveryStore, Workspace};
use std::{
    fs,
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};

fn budget(cancelled: &AtomicBool) -> Budget<'_> {
    Budget {
        cancelled,
        deadline: Instant::now() + Duration::from_secs(5),
    }
}
fn recoveries(files: &support::Fixture) -> RecoveryStore {
    RecoveryStore::in_store(&jecode::state::Store::in_home(&files.home()).unwrap()).unwrap()
}
#[test]
fn exact_edit_is_inert_until_apply_and_retains_original_bytes() {
    let files = support::Fixture::new();
    let original = "fn main() {\r\n\tlet café = 2;\r\n}\r\n";
    files.write("src/main.rs", original);
    let ws = Workspace::open(&files.0).unwrap();
    let cancel = AtomicBool::new(false);
    let change = ws
        .prepare_edit("src/main.rs", "café = 2", "café = 3", &budget(&cancel))
        .unwrap();
    assert!(change.preview().diff.contains("- \\tlet café = 2;\\r"));
    assert!(change.preview().diff.contains("+ \\tlet café = 3;\\r"));
    assert_eq!((change.preview().added, change.preview().removed), (1, 1));
    assert_eq!(
        fs::read_to_string(files.0.join("src/main.rs")).unwrap(),
        original
    );
    let applied = ws.apply(change, &budget(&cancel), &recoveries(&files), None, "test");
    if applied
        .as_ref()
        .is_err_and(|e| files.unsupported_host_filesystem(&e.to_string()))
    {
        assert_eq!(
            fs::read_to_string(files.0.join("src/main.rs")).unwrap(),
            original
        );
        assert_eq!(fs::read_dir(files.0.join("src")).unwrap().count(), 1);
        return;
    }
    let applied = applied.unwrap();
    assert_eq!(
        ws.read("src/main.rs", &budget(&cancel)).unwrap(),
        original.replace("= 2", "= 3")
    );
    assert_eq!(
        fs::read_to_string(
            recoveries(&files)
                .root()
                .join(format!("{}.before", applied.recovery.unwrap()))
        )
        .unwrap(),
        original
    );
    assert_eq!(ws.list("src", &budget(&cancel)).unwrap().omitted, 0);
}
#[test]
fn create_missing_empty_file_and_reject_overwrite_or_missing_parent() {
    let files = support::Fixture::new();
    let ws = Workspace::open(&files.0).unwrap();
    let cancel = AtomicBool::new(false);
    let change = ws.prepare_create("empty", "", &budget(&cancel)).unwrap();
    assert!(!files.0.join("empty").exists());
    let applied = ws.apply(change, &budget(&cancel), &recoveries(&files), None, "test");
    if applied
        .as_ref()
        .is_err_and(|e| files.unsupported_host_filesystem(&e.to_string()))
    {
        assert!(!files.0.join("empty").exists());
        assert_eq!(fs::read_dir(&files.0).unwrap().count(), 0);
        return;
    }
    assert!(applied.unwrap().recovery.is_none());
    assert_eq!(fs::read(files.0.join("empty")).unwrap(), b"");
    assert!(
        ws.prepare_create("empty", "overwrite", &budget(&cancel))
            .is_err()
    );
    assert!(
        ws.prepare_create("missing/child", "x", &budget(&cancel))
            .is_err()
    );
    let fill = ws
        .prepare_edit("empty", "", "filled\n", &budget(&cancel))
        .unwrap();
    let applied = ws
        .apply(fill, &budget(&cancel), &recoveries(&files), None, "test")
        .unwrap();
    assert_eq!(
        fs::read_to_string(files.0.join("empty")).unwrap(),
        "filled\n"
    );
    assert_eq!(
        fs::read(
            recoveries(&files)
                .root()
                .join(format!("{}.before", applied.recovery.unwrap()))
        )
        .unwrap(),
        b""
    );
}

#[cfg(unix)]
#[test]
fn links_are_rejected_for_both_change_kinds() {
    let files = support::Fixture::new();
    files.write("original", "old");
    std::os::unix::fs::symlink("original", files.0.join("link")).unwrap();
    std::os::unix::fs::symlink("absent", files.0.join("dangling")).unwrap();
    let ws = Workspace::open(&files.0).unwrap();
    let cancel = AtomicBool::new(false);
    assert!(
        ws.prepare_edit("link", "old", "new", &budget(&cancel))
            .is_err()
    );
    assert!(
        ws.prepare_create("dangling", "new", &budget(&cancel))
            .is_err()
    );
    assert_eq!(fs::read_to_string(files.0.join("original")).unwrap(), "old");
    assert!(!files.0.join("absent").exists());
}

#[cfg(windows)]
#[test]
fn alternate_streams_and_readonly_files_are_not_recreated_with_lost_metadata() {
    let files = support::Fixture::new();
    files.write("streams", "old");
    files.write("streams:extra", "preserve");
    files.write("readonly", "old");
    let path = files.0.join("readonly");
    let original_permissions = fs::metadata(&path).unwrap().permissions();
    let mut perms = original_permissions.clone();
    perms.set_readonly(true);
    fs::set_permissions(&path, perms).unwrap();
    let ws = Workspace::open(&files.0).unwrap();
    let cancel = AtomicBool::new(false);
    let rejected = ["streams", "readonly"]
        .map(|p| ws.prepare_edit(p, "old", "new", &budget(&cancel)).is_err());
    fs::set_permissions(&path, original_permissions).unwrap();
    assert_eq!(rejected, [true, true]);
    assert_eq!(
        fs::read_to_string(files.0.join("streams:extra")).unwrap(),
        "preserve"
    );
}
#[test]
fn stale_content_identity_creation_and_cancellation_preserve_disk() {
    let files = support::Fixture::new();
    files.write("file", "old");
    let ws = Workspace::open(&files.0).unwrap();
    let cancel = AtomicBool::new(false);
    let change = ws
        .prepare_edit("file", "old", "new", &budget(&cancel))
        .unwrap();
    files.write("file", "other editor");
    assert!(
        ws.apply(change, &budget(&cancel), &recoveries(&files), None, "test")
            .is_err()
    );
    assert_eq!(
        fs::read_to_string(files.0.join("file")).unwrap(),
        "other editor"
    );
    let change = ws
        .prepare_edit("file", "other editor", "new", &budget(&cancel))
        .unwrap();
    fs::rename(files.0.join("file"), files.0.join("moved")).unwrap();
    files.write("file", "other editor");
    assert!(
        ws.apply(change, &budget(&cancel), &recoveries(&files), None, "test")
            .is_err()
    );
    let change = ws
        .prepare_create("new", "proposed", &budget(&cancel))
        .unwrap();
    files.write("new", "created meanwhile");
    assert!(
        ws.apply(change, &budget(&cancel), &recoveries(&files), None, "test")
            .is_err()
    );
    assert_eq!(
        fs::read_to_string(files.0.join("new")).unwrap(),
        "created meanwhile"
    );
    let change = ws
        .prepare_edit("file", "other editor", "new", &budget(&cancel))
        .unwrap();
    cancel.store(true, Ordering::Release);
    assert!(
        ws.apply(change, &budget(&cancel), &recoveries(&files), None, "test")
            .is_err()
    );
    assert_eq!(fs::read_dir(&files.0).unwrap().count(), 3);
}
#[test]
fn ambiguous_binary_and_hidden_changes_fail_while_long_valid_changes_prepare() {
    let files = support::Fixture::new();
    files.write("file", "aaa");
    files.write("binary", b"a\0b");
    let ws = Workspace::open(&files.0).unwrap();
    let cancel = AtomicBool::new(false);
    for (path, old, new) in [
        ("file", "aa", "b"),
        ("file", "absent", "b"),
        ("file", "", "x"),
        ("file", "aaa", "aaa"),
        ("file", "aaa", "x\u{202e}"),
        ("binary", "a", "b"),
    ] {
        assert!(ws.prepare_edit(path, old, new, &budget(&cancel)).is_err());
    }
    for path in [
        "../escape",
        ".env",
        "target/new",
        "file:stream",
        ".",
        "x\u{202e}",
    ] {
        assert!(ws.prepare_create(path, "x", &budget(&cancel)).is_err());
    }
    let large = ws
        .prepare_create("large", &"x".repeat(32769), &budget(&cancel))
        .unwrap();
    assert_eq!(large.preview().added, 1);
    drop(large);
    let lines = ws
        .prepare_create("lines", &"x\n".repeat(401), &budget(&cancel))
        .unwrap();
    assert_eq!(lines.preview().added, 401);
    assert!(lines.preview().omitted_lines > 0);
    drop(lines);
    let change = ws
        .prepare_edit("file", "aaa", "b\n", &budget(&cancel))
        .unwrap();
    assert!(change.preview().diff.contains("No newline at end of file"));
    assert_eq!(fs::read_dir(&files.0).unwrap().count(), 2);
}
#[test]
fn hardlinks_are_not_edited_and_replaced_parent_invalidates_proposal() {
    let files = support::Fixture::new();
    files.write("src/file", "original");
    fs::hard_link(files.0.join("src/file"), files.0.join("link")).unwrap();
    let ws = Workspace::open(&files.0).unwrap();
    let cancel = AtomicBool::new(false);
    assert!(
        ws.prepare_edit("src/file", "original", "new", &budget(&cancel))
            .is_err()
    );
    fs::remove_file(files.0.join("link")).unwrap();
    let change = ws.prepare_create("src/new", "x", &budget(&cancel)).unwrap();
    fs::rename(files.0.join("src"), files.0.join("old-src")).unwrap();
    fs::create_dir(files.0.join("src")).unwrap();
    assert!(
        ws.apply(change, &budget(&cancel), &recoveries(&files), None, "test")
            .is_err()
    );
    assert!(!files.0.join("src/new").exists());
}
