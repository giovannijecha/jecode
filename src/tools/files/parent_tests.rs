//! The missing parent directories of a new file.

use super::*;
use crate::test_support::Directory;

fn arguments(path: &str, content: &str) -> Value {
    Value::object([
        ("path", Value::string(path)),
        ("content", Value::string(content)),
    ])
}

fn workspace(directory: &Directory) -> PathBuf {
    let workspace = directory.path().join("workspace");
    fs::create_dir(&workspace).unwrap();
    fs::canonicalize(workspace).unwrap()
}

#[test]
fn write_creates_the_missing_parents_of_a_new_file() {
    let directory = Directory::new();
    let root = workspace(&directory);
    let (result, change) = write_with_change(
        &root,
        &arguments("src/core/todo.js", "one\n"),
        None,
        &Cancellation::default(),
    )
    .unwrap();
    assert_eq!(result.get("bytes_written"), Some(&Value::number(4)));
    assert!(change.is_some());
    assert_eq!(
        fs::read_to_string(root.join("src/core/todo.js")).unwrap(),
        "one\n"
    );
    // `..` after names that exist resolves as it does anywhere else.
    write(&root, &arguments("src/core/../storage/schema.js", "two\n")).unwrap();
    assert_eq!(
        fs::read_to_string(root.join("src/storage/schema.js")).unwrap(),
        "two\n"
    );
}

#[test]
fn concurrent_writes_share_the_parents_they_create() {
    let directory = Directory::new();
    let root = workspace(&directory);
    // Each round contends for new folders; declared parallel calls run four at once.
    for round in 0..25 {
        let start = std::sync::Barrier::new(4);
        std::thread::scope(|scope| {
            for index in 0..4 {
                let (root, start) = (&root, &start);
                scope.spawn(move || {
                    let path = format!("{round}/core/{index}.js");
                    start.wait();
                    let result = write(root, &arguments(&path, "text\n")).unwrap();
                    assert!(result.get("error").is_none(), "{result:?}");
                });
            }
        });
        let folder = root.join(round.to_string()).join("core");
        assert_eq!(fs::read_dir(folder).unwrap().count(), 4);
    }
}

#[test]
fn missing_parents_are_never_created_outside_the_root() {
    let directory = Directory::new();
    let root = workspace(&directory);
    let outside = fs::canonicalize(directory.path())
        .unwrap()
        .join("made")
        .join("new.txt")
        .to_string_lossy()
        .into_owned();
    for path in [
        "../made/new.txt",
        "missing/../../made/new.txt",
        outside.as_str(),
    ] {
        assert!(write(&root, &arguments(path, "x")).is_err(), "{path}");
        assert!(!directory.path().join("made").exists(), "{path}");
        assert!(!root.join("missing").exists(), "{path}");
    }
}

#[test]
fn a_refused_write_creates_no_parents() {
    let directory = Directory::new();
    let root = workspace(&directory);
    let cancellation = Cancellation::default();
    cancellation.cancel();
    assert!(
        write_with_change(
            &root,
            &arguments("cancelled/new.txt", "x"),
            None,
            &cancellation
        )
        .is_err()
    );
    let binary = write(&root, &arguments("binary/new.txt", "\0")).unwrap();
    assert!(binary.get("error").is_some(), "{binary:?}");
    fs::write(root.join("source"), "text").unwrap();
    let nested = write(&root, &arguments("source/child/new.txt", "x"));
    assert!(nested.is_err() || nested.unwrap().get("error").is_some());
    assert_eq!(fs::read_to_string(root.join("source")).unwrap(), "text");
    assert_eq!(fs::read_dir(&root).unwrap().count(), 1);
}

#[cfg(unix)]
#[test]
fn missing_parents_cannot_pass_through_links() {
    use std::os::unix::fs::symlink;
    let directory = Directory::new();
    let root = workspace(&directory);
    let outside = directory.path().join("outside");
    fs::create_dir(&outside).unwrap();
    symlink(&outside, root.join("link")).unwrap();
    symlink(directory.path().join("gone"), root.join("broken")).unwrap();
    for path in [
        "link/new/file.txt",
        "broken/new/file.txt",
        "missing/../link/new/file.txt",
    ] {
        assert!(write(&root, &arguments(path, "x")).is_err(), "{path}");
    }
    assert_eq!(fs::read_dir(&outside).unwrap().count(), 0);
    assert!(!directory.path().join("gone").exists());
    assert!(!root.join("missing").exists());
}

#[cfg(windows)]
#[test]
fn missing_parents_cannot_pass_through_junctions() {
    let directory = Directory::new();
    let root = workspace(&directory);
    let base = fs::canonicalize(directory.path()).unwrap();
    let outside = base.join("outside");
    let gone = base.join("gone");
    for (name, target) in [("link", &outside), ("broken", &gone)] {
        fs::create_dir(target).unwrap();
        let created = std::process::Command::new("cmd.exe")
            .args(["/c", "mklink", "/J"])
            .arg(root.join(name))
            .arg(target)
            .output()
            .unwrap();
        assert!(created.status.success(), "{created:?}");
    }
    fs::remove_dir(&gone).unwrap();
    for path in [
        "link/new/file.txt",
        "broken/new/file.txt",
        "missing/../link/new/file.txt",
    ] {
        assert!(write(&root, &arguments(path, "x")).is_err(), "{path}");
    }
    assert_eq!(fs::read_dir(&outside).unwrap().count(), 0);
    assert!(!gone.exists());
    assert!(!root.join("missing").exists());
}
