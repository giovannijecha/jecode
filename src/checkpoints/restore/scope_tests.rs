use super::*;
use crate::test_support::Directory;
use std::fs;

#[test]
fn implicit_scope_does_not_infer_absence_for_generated_content() {
    for name in ["target/owned.txt", "src/node_modules/owned.txt"] {
        let root = Directory::new();
        let home = Directory::new();
        let path = root.path().join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, b"not captured").unwrap();
        let store = Store::open(home.path(), root.path()).unwrap();
        let cancel = Cancellation::default();
        let reference = store
            .capture("bash", "test", &[".".into()], &cancel)
            .unwrap()
            .reference
            .unwrap();
        let requested = [name.into()];
        let inspection = store
            .inspect(&reference, Some(&requested), &cancel)
            .unwrap();
        assert!(inspection.selected.is_empty(), "{inspection:?}");
        let report = store
            .restore(&reference, Some(&requested), "latest", false, &cancel)
            .unwrap();
        assert!(report.restored.is_empty());
        assert!(report.undo.is_none());
        assert_eq!(fs::read(&path).unwrap(), b"not captured");
    }
}

#[test]
fn explicit_generated_scope_still_covers_its_eligible_descendants() {
    let root = Directory::new();
    let home = Directory::new();
    fs::create_dir(root.path().join("target")).unwrap();
    fs::write(root.path().join("target/owned.txt"), b"before").unwrap();
    let store = Store::open(home.path(), root.path()).unwrap();
    let cancel = Cancellation::default();
    let reference = store
        .capture("bash", "test", &["target".into()], &cancel)
        .unwrap()
        .reference
        .unwrap();
    fs::write(root.path().join("target/owned.txt"), b"after").unwrap();
    fs::write(root.path().join("target/new.txt"), b"new").unwrap();
    let requested = ["target/new.txt".into()];
    let report = store
        .restore(&reference, Some(&requested), "latest", false, &cancel)
        .unwrap();
    assert_eq!(report.restored.len(), 1);
    assert!(!root.path().join("target/new.txt").exists());
    assert_eq!(
        fs::read(root.path().join("target/owned.txt")).unwrap(),
        b"after"
    );
}

#[test]
fn git_ignored_directories_are_skipped_and_never_proven_absent() {
    let root = Directory::new();
    let home = Directory::new();
    fs::create_dir_all(root.path().join(".git")).unwrap();
    fs::create_dir(root.path().join("build")).unwrap();
    fs::write(root.path().join(".gitignore"), b"build/\n.env\n").unwrap();
    fs::write(root.path().join(".env"), b"secret").unwrap();
    fs::write(root.path().join("build/out.bin"), b"old").unwrap();
    let store = Store::open(home.path(), root.path()).unwrap();
    let cancel = Cancellation::default();
    let reference = store
        .capture("bash", "test", &[".".into()], &cancel)
        .unwrap()
        .reference
        .unwrap();
    let snapshot = store.load(&reference, &cancel).unwrap().snapshot;
    assert_eq!(snapshot.ignored, ["build"]);
    assert!(snapshot.entries.contains_key(".env"));
    assert!(
        !snapshot
            .entries
            .keys()
            .any(|path| path.starts_with("build"))
    );
    // Un-ignoring the directory later must not widen the absence proof.
    fs::write(root.path().join(".gitignore"), b"").unwrap();
    fs::write(root.path().join(".env"), b"changed").unwrap();
    fs::write(root.path().join("build/new.bin"), b"new").unwrap();
    fs::write(root.path().join("new.txt"), b"new").unwrap();
    store
        .restore(&reference, None, "latest", false, &cancel)
        .unwrap();
    assert!(!root.path().join("new.txt").exists());
    assert_eq!(fs::read(root.path().join(".env")).unwrap(), b"secret");
    assert_eq!(fs::read(root.path().join("build/new.bin")).unwrap(), b"new");
    assert_eq!(fs::read(root.path().join("build/out.bin")).unwrap(), b"old");
}

#[test]
fn restore_keeps_the_captured_scope_when_ignore_rules_change() {
    let root = Directory::new();
    let home = Directory::new();
    fs::create_dir(root.path().join("src")).unwrap();
    fs::write(root.path().join(".gitignore"), b"").unwrap();
    fs::write(root.path().join("src/main.rs"), b"before").unwrap();
    let store = Store::open(home.path(), root.path()).unwrap();
    let cancel = Cancellation::default();
    let reference = store
        .capture("bash", "test", &[".".into()], &cancel)
        .unwrap()
        .reference
        .unwrap();
    fs::write(root.path().join(".gitignore"), b"src/\n").unwrap();
    fs::write(root.path().join("src/main.rs"), b"after").unwrap();
    fs::write(root.path().join("src/new.rs"), b"new").unwrap();
    let report = store
        .restore(&reference, None, "latest", false, &cancel)
        .unwrap();
    assert!(report.failed.is_empty(), "{report:?}");
    assert!(!root.path().join("src/new.rs").exists());
    assert_eq!(
        fs::read(root.path().join("src/main.rs")).unwrap(),
        b"before"
    );
}

#[test]
fn absent_ignored_directories_cannot_be_selected_as_protected_absence() {
    for explicit in [false, true] {
        let root = Directory::new();
        let home = Directory::new();
        fs::write(root.path().join(".gitignore"), b"build/\n").unwrap();
        let store = Store::open(home.path(), root.path()).unwrap();
        let cancel = Cancellation::default();
        let reference = store
            .capture("bash", "test", &[".".into()], &cancel)
            .unwrap()
            .reference
            .unwrap();
        drop(store);
        fs::write(root.path().join(".gitignore"), b"").unwrap();
        fs::create_dir(root.path().join("build")).unwrap();
        fs::write(root.path().join("build/out.bin"), b"unprotected").unwrap();
        let store = Store::open(home.path(), root.path()).unwrap();
        let requested = ["build/out.bin".into()];
        let report = store
            .restore(
                &reference,
                explicit.then_some(requested.as_slice()),
                "latest",
                false,
                &cancel,
            )
            .unwrap();
        assert!(report.failed.is_empty(), "{report:?}");
        assert_eq!(
            fs::read(root.path().join("build/out.bin")).unwrap(),
            b"unprotected"
        );
    }
}

#[test]
fn explicit_ignored_scope_overrides_the_implicit_scope_exclusion() {
    let root = Directory::new();
    let home = Directory::new();
    fs::write(root.path().join(".gitignore"), b"build/\n").unwrap();
    fs::create_dir(root.path().join("build")).unwrap();
    fs::write(root.path().join("build/old.bin"), b"before").unwrap();
    let store = Store::open(home.path(), root.path()).unwrap();
    let cancel = Cancellation::default();
    let reference = store
        .capture("bash", "test", &[".".into(), "build".into()], &cancel)
        .unwrap()
        .reference
        .unwrap();
    fs::write(root.path().join("build/new.bin"), b"new").unwrap();
    let report = store
        .restore(&reference, None, "latest", false, &cancel)
        .unwrap();
    assert!(report.failed.is_empty(), "{report:?}");
    assert!(!root.path().join("build/new.bin").exists());
    assert_eq!(
        fs::read(root.path().join("build/old.bin")).unwrap(),
        b"before"
    );
}
