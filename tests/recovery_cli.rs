#[path = "support/workspace.rs"]
mod support;
use jecode::{
    state::Store,
    workspace::{Access, Budget, RecoveryStore, Workspace},
};
use std::{
    fs,
    process::{Command, Output},
    sync::atomic::AtomicBool,
    time::{Duration, Instant},
};

fn command(files: &support::Fixture, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_jecode"))
        .args(args)
        .env("USERPROFILE", files.home())
        .env("HOME", files.home())
        .output()
        .unwrap()
}

#[test]
fn cli_inspects_exact_original_and_restores_after_reopening_state() {
    let files = support::Fixture::new();
    files.write("index.html", "original\r\n");
    let store = Store::in_home(&files.home()).unwrap();
    let recoveries = RecoveryStore::in_store(&store).unwrap();
    let ws = Workspace::open(&files.0).unwrap();
    let cancelled = AtomicBool::new(false);
    let budget = Budget {
        cancelled: &cancelled,
        deadline: Instant::now() + Duration::from_secs(20),
    };
    let change = ws
        .prepare_edit("index.html", "original", "changed", &budget)
        .unwrap();
    let id = ws
        .apply(change, &budget, &recoveries, Some("s-cli"), "edit-1")
        .unwrap()
        .recovery
        .unwrap();
    drop(ws);
    drop(recoveries);
    drop(store);
    let workspace = files.0.to_str().unwrap();
    let list = command(&files, &["--workspace", workspace, "recover", "list"]);
    assert!(
        list.status.success(),
        "{}",
        String::from_utf8_lossy(&list.stderr)
    );
    assert!(String::from_utf8(list.stdout).unwrap().contains(&id));
    let show = command(&files, &["recover", "show", &id, "--workspace", workspace]);
    assert!(
        show.status.success(),
        "{}",
        String::from_utf8_lossy(&show.stderr)
    );
    let shown = String::from_utf8(show.stdout).unwrap();
    assert!(shown.contains("Session: s-cli"));
    assert!(shown.contains("Operation: edit-1"));
    assert!(shown.contains("Original integrity: verified"));
    assert!(shown.contains("Result integrity: verified"));
    assert!(shown.contains("Current target: published result"));
    let cat = command(&files, &["recover", "cat", &id, "--workspace", workspace]);
    assert!(cat.status.success());
    assert_eq!(cat.stdout, b"original\r\n");
    let restore = command(
        &files,
        &["recover", "restore", &id, "--workspace", workspace],
    );
    assert!(
        restore.status.success(),
        "{}",
        String::from_utf8_lossy(&restore.stderr)
    );
    assert_eq!(
        fs::read(files.0.join("index.html")).unwrap(),
        b"original\r\n"
    );
    assert_eq!(fs::read_dir(&files.0).unwrap().count(), 1);
}

#[test]
fn cli_rejects_conflict_and_other_workspace_without_touching_target() {
    let files = support::Fixture::new();
    fs::create_dir(files.0.join("one")).unwrap();
    fs::create_dir(files.0.join("two")).unwrap();
    files.write("one/index.html", "old");
    let recoveries = RecoveryStore::in_store(&Store::in_home(&files.home()).unwrap()).unwrap();
    let ws = Workspace::open(&files.0.join("one"))
        .unwrap()
        .with_access(Access::Local);
    let cancelled = AtomicBool::new(false);
    let budget = Budget {
        cancelled: &cancelled,
        deadline: Instant::now() + Duration::from_secs(20),
    };
    let change = ws
        .prepare_edit("index.html", "old", "new", &budget)
        .unwrap();
    let id = ws
        .apply(change, &budget, &recoveries, Some("s-1"), "edit")
        .unwrap()
        .recovery
        .unwrap();
    let two = files.0.join("two");
    let other = command(
        &files,
        &["--workspace", two.to_str().unwrap(), "recover", "show", &id],
    );
    assert!(!other.status.success());
    assert!(
        String::from_utf8(other.stderr)
            .unwrap()
            .contains("another selected workspace")
    );
    files.write("one/index.html", "intervening user change");
    let one = files.0.join("one");
    let conflict = command(
        &files,
        &[
            "--workspace",
            one.to_str().unwrap(),
            "recover",
            "restore",
            &id,
        ],
    );
    assert!(!conflict.status.success());
    assert!(
        String::from_utf8(conflict.stderr)
            .unwrap()
            .contains("restoration refused")
    );
    assert_eq!(
        fs::read(one.join("index.html")).unwrap(),
        b"intervening user change"
    );
}
