#[path = "support/workspace.rs"]
mod support;
use jecode::{
    json::{self, Value},
    tools::Prepared,
    workspace::{Access, Budget, Error, Workspace},
};
use std::{
    sync::atomic::AtomicBool,
    time::{Duration, Instant},
};
use support::Fixture;

fn budget(cancelled: &AtomicBool) -> Budget<'_> {
    Budget {
        cancelled,
        deadline: Instant::now() + Duration::from_secs(10),
    }
}
fn path(path: &std::path::Path) -> String {
    path.to_str().unwrap().replace('\\', "/")
}

#[test]
fn local_paths_read_list_and_search_a_sibling_without_changing_the_base() {
    let files = Fixture::new();
    files.write("a/readme.txt", "project A");
    files.write("b/readme.txt", "project B needle");
    files.write("b/target/report.txt", "generated evidence");
    let workspace = Workspace::open(&files.0.join("a"))
        .unwrap()
        .with_access(Access::Local);
    let cancelled = AtomicBool::new(false);
    let budget = budget(&cancelled);
    for candidate in [
        "../b/readme.txt".into(),
        path(&files.0.join("b/readme.txt")),
    ] {
        assert_eq!(
            workspace.read(&candidate, &budget).unwrap(),
            "project B needle"
        );
    }
    assert_eq!(
        workspace.read("./readme.txt", &budget).unwrap(),
        "project A"
    );
    assert_eq!(
        workspace.read("../b/target/report.txt", &budget).unwrap(),
        "generated evidence"
    );
    let list = workspace.list("../b", &budget).unwrap();
    assert_eq!(list.entries.len(), 1);
    assert_eq!(list.omitted, 1);
    let tool = Prepared::parse(
        "search_text",
        &json::object([
            ("path", Value::String(path(&files.0.join("b")))),
            ("query", Value::String("needle".into())),
        ]),
    )
    .unwrap();
    let output = tool.execute(&workspace, &budget);
    assert!(!output.failed);
    let result = json::parse(&output.text, Default::default()).unwrap();
    assert_eq!(
        result.get("matches").and_then(Value::array).unwrap().len(),
        1
    );
    assert_eq!(workspace.read("readme.txt", &budget).unwrap(), "project A");
}

#[test]
fn old_profile_and_private_paths_do_not_inherit_local_access() {
    let files = Fixture::new();
    files.write("a/file", "A");
    files.write("b/file", "B");
    files.write("b/.jecode/v1/credentials.json", "fake secret");
    files.write("b/tokens.json", "fake secret");
    let workspace = Workspace::open(&files.0.join("a")).unwrap();
    let cancelled = AtomicBool::new(false);
    let budget = budget(&cancelled);
    assert_eq!(workspace.read("../b/file", &budget), Err(Error::Path));
    assert_eq!(
        workspace.read(&path(&files.0.join("b/file")), &budget),
        Err(Error::Path)
    );
    let local = workspace.with_access(Access::Local);
    for target in [
        "../b/.jecode/v1/credentials.json",
        "../b/tokens.json",
        "../b/.jecode/../file",
    ] {
        assert_eq!(local.read(target, &budget), Err(Error::Excluded));
    }
    assert_eq!(
        local.read(&path(&files.0.join("b/tokens.json")), &budget),
        Err(Error::Excluded)
    );
    assert!(
        local
            .prepare_create("../b/auth.json", "x", &budget)
            .is_err()
    );
    for target in [
        "../b/file\u{202e}",
        "file:stream",
        r"\\server\share\file",
        r"C:relative",
    ] {
        assert!(local.read(target, &budget).is_err());
    }
}

#[test]
fn external_changes_have_absolute_previews_and_reject_stale_content() {
    let files = Fixture::new();
    files.write("a/file", "A");
    files.write("b/file", "old\n");
    let local = Workspace::open(&files.0.join("a"))
        .unwrap()
        .with_access(Access::Local);
    let cancelled = AtomicBool::new(false);
    let budget = budget(&cancelled);
    let edit = local
        .prepare_edit("../b/file", "old", "new", &budget)
        .unwrap();
    assert_eq!(edit.preview().path, path(&files.0.join("b/file")));
    assert_eq!(
        std::fs::read_to_string(files.0.join("b/file")).unwrap(),
        "old\n"
    );
    files.write("b/file", "changed elsewhere\n");
    let recoveries = jecode::workspace::RecoveryStore::in_store(
        &jecode::state::Store::in_home(&files.home()).unwrap(),
    )
    .unwrap();
    assert!(
        local
            .apply(edit, &budget, &recoveries, None, "test")
            .is_err()
    );
    assert_eq!(
        std::fs::read_to_string(files.0.join("b/file")).unwrap(),
        "changed elsewhere\n"
    );
    let create = local
        .prepare_create("../b/new.txt", "new file", &budget)
        .unwrap();
    assert!(!files.0.join("b/new.txt").exists());
    match local.apply(create, &budget, &recoveries, None, "test") {
        Ok(_) => assert_eq!(local.read("../b/new.txt", &budget).unwrap(), "new file"),
        Err(e) if files.unsupported_host_filesystem(&e.to_string()) => {}
        Err(e) => panic!("{e}"),
    }
}

#[cfg(unix)]
#[test]
fn external_symlinks_are_not_followed_even_when_the_target_is_readable() {
    use std::os::unix::fs::symlink;
    let files = Fixture::new();
    files.write("a/file", "A");
    files.write("b/file", "B");
    symlink("b", files.0.join("alias")).unwrap();
    let local = Workspace::open(&files.0.join("a"))
        .unwrap()
        .with_access(Access::Local);
    assert_eq!(
        local.read("../alias/file", &budget(&AtomicBool::new(false))),
        Err(Error::Unavailable)
    );
}

#[cfg(windows)]
#[test]
fn windows_native_and_verbatim_drive_paths_address_the_same_file() {
    let files = Fixture::new();
    files.write("a/file", "A");
    files.write("b/file", "B");
    let local = Workspace::open(&files.0.join("a"))
        .unwrap()
        .with_access(Access::Local);
    let cancelled = AtomicBool::new(false);
    let budget = budget(&cancelled);
    for target in [
        files.0.join("b/file"),
        files.0.join("b/file").canonicalize().unwrap(),
    ] {
        assert_eq!(local.read(target.to_str().unwrap(), &budget).unwrap(), "B");
    }
}
