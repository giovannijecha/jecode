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
