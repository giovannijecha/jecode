#[cfg(unix)]
use super::super::files;
use super::*;
use crate::test_support::Directory;
use std::fs;
use std::path::PathBuf;

struct Fixture(Directory);
impl Fixture {
    fn new() -> Self {
        let directory = Directory::new();
        fs::create_dir(directory.path().join("work")).unwrap();
        fs::create_dir(directory.path().join("home")).unwrap();
        Self(directory)
    }
    fn work(&self) -> PathBuf {
        self.0.path().join("work")
    }
    fn store(&self) -> std::sync::Arc<Store> {
        Store::open(&self.0.path().join("home"), &self.work()).unwrap()
    }
}

fn captured(store: &Store, names: &[&str]) -> String {
    store
        .capture(
            "write",
            "test",
            &names.iter().map(|name| (*name).into()).collect::<Vec<_>>(),
            &Cancellation::default(),
        )
        .unwrap()
        .reference
        .unwrap()
}

#[test]
fn simple_file_restore_uses_its_path_lease() {
    let fixture = Fixture::new();
    fs::write(fixture.work().join("note"), b"before").unwrap();
    let store = fixture.store();
    let reference = captured(&store, &["note"]);
    let loaded = store.load(&reference, &Cancellation::default()).unwrap();
    assert_eq!(
        lease_paths(&loaded.snapshot, &store.root, None),
        Some(vec!["note".into()])
    );

    let reference = captured(&store, &[""]);
    let loaded = store.load(&reference, &Cancellation::default()).unwrap();
    assert_eq!(lease_paths(&loaded.snapshot, &store.root, None), None);
    assert_eq!(
        lease_paths(&loaded.snapshot, &store.root, Some(&["note".into()])),
        Some(vec!["note".into()])
    );

    fs::create_dir(fixture.work().join("folder")).unwrap();
    let reference = captured(&store, &["folder"]);
    let loaded = store.load(&reference, &Cancellation::default()).unwrap();
    assert_eq!(
        lease_paths(&loaded.snapshot, &store.root, Some(&["folder".into()])),
        None
    );
}

#[test]
fn file_becoming_a_directory_after_narrow_lease_restores_its_children() {
    let fixture = Fixture::new();
    let file = fixture.work().join("note");
    fs::write(&file, b"saved file").unwrap();
    let store = fixture.store();
    let reference = captured(&store, &[""]);
    fs::write(&file, b"current file").unwrap();
    let changed = file.clone();
    AFTER_LEASE.with(|hook| {
        *hook.borrow_mut() = Some(Box::new(move || {
            fs::remove_file(&changed).unwrap();
            fs::create_dir(&changed).unwrap();
            fs::write(changed.join("child"), b"new child").unwrap();
        }));
    });
    let report = store
        .restore(
            &reference,
            Some(&["note".into()]),
            "latest",
            false,
            &Cancellation::default(),
        )
        .unwrap();
    assert!(report.failed.is_empty(), "{:?}", report.failed);
    assert_eq!(fs::read(file).unwrap(), b"saved file");
    assert_eq!(report.undo.unwrap().paths, ["note", "note/child"]);
}

#[test]
fn exact_file_restore_and_undo_only_changed_path() {
    let fixture = Fixture::new();
    let file = fixture.work().join("note.txt");
    fs::write(&file, b"before").unwrap();
    let store = fixture.store();
    let reference = captured(&store, &["note.txt"]);
    fs::write(&file, b"after").unwrap();
    let inspected = store
        .inspect(&reference, None, &Cancellation::default())
        .unwrap();
    assert_eq!(inspected.selected[0].action, Some(Action::Replace));
    let preview = store
        .restore(
            &reference,
            None,
            &inspected.expected,
            true,
            &Cancellation::default(),
        )
        .unwrap();
    assert_eq!(preview.planned, [("note.txt".into(), Action::Replace)]);
    assert!(preview.restored.is_empty());
    assert!(preview.undo.is_none());
    assert_eq!(fs::read(&file).unwrap(), b"after");
    let report = store
        .restore(
            &reference,
            None,
            &inspected.expected,
            false,
            &Cancellation::default(),
        )
        .unwrap();
    assert_eq!(fs::read(&file).unwrap(), b"before");
    assert_eq!(report.restored, [("note.txt".into(), Action::Replace)]);
    let undo = report.undo.unwrap();
    assert_eq!(undo.paths, ["note.txt"]);
    let restored = store
        .restore(
            &undo.reference,
            Some(&undo.paths),
            &undo.expected,
            false,
            &Cancellation::default(),
        )
        .unwrap();
    assert!(restored.failed.is_empty());
    assert_eq!(fs::read(file).unwrap(), b"after");
}

#[test]
fn created_and_deleted_files_restore_from_one_complete_scope() {
    let fixture = Fixture::new();
    fs::write(fixture.work().join("deleted"), b"old").unwrap();
    let store = fixture.store();
    let reference = captured(&store, &[""]);
    fs::remove_file(fixture.work().join("deleted")).unwrap();
    fs::write(fixture.work().join("created"), b"new").unwrap();
    let report = store
        .restore(
            &reference,
            Some(&[".".into()]),
            "latest",
            false,
            &Cancellation::default(),
        )
        .unwrap();
    assert!(report.failed.is_empty());
    assert_eq!(fs::read(fixture.work().join("deleted")).unwrap(), b"old");
    assert!(!fixture.work().join("created").exists());
    assert_eq!(report.undo.unwrap().paths.len(), 2);
}

#[test]
fn missing_protected_directory_restores_new_tree_and_undoes() {
    let fixture = Fixture::new();
    let store = fixture.store();
    let reference = captured(&store, &["newdir/"]);
    let saved = store.load(&reference, &Cancellation::default()).unwrap();
    assert_eq!(saved.snapshot.roots, ["newdir"]);
    assert!(matches!(saved.snapshot.entries["newdir"], State::Absent));
    drop(saved);
    let directory = fixture.work().join("newdir");
    fs::create_dir(&directory).unwrap();
    fs::write(directory.join("child"), b"created").unwrap();
    let report = store
        .restore(&reference, None, "latest", false, &Cancellation::default())
        .unwrap();
    assert!(report.failed.is_empty(), "{:?}", report.failed);
    assert!(!directory.exists());
    let undo = report.undo.unwrap();
    assert_eq!(undo.paths, ["newdir", "newdir/child"]);
    let undone = store
        .restore(
            &undo.reference,
            Some(&undo.paths),
            &undo.expected,
            false,
            &Cancellation::default(),
        )
        .unwrap();
    assert!(undone.failed.is_empty(), "{:?}", undone.failed);
    assert_eq!(fs::read(directory.join("child")).unwrap(), b"created");
}

#[test]
fn partial_scope_does_not_infer_absence_of_unsaved_file() {
    let fixture = Fixture::new();
    let store = fixture.store();
    fs::write(fixture.work().join("known"), b"before").unwrap();
    let reference = captured(&store, &["known"]);
    let loaded = store.load(&reference, &Cancellation::default()).unwrap();
    let mut snapshot = loaded.snapshot.clone();
    snapshot
        .partial
        .push((String::new(), "Scope incomplete".into()));
    fs::write(fixture.work().join("new"), b"keep").unwrap();
    let inspected = inspect_loaded(&store, &snapshot, None, &Cancellation::default()).unwrap();
    assert!(inspected.selected.iter().all(|entry| entry.path != "new"));
    assert_eq!(fs::read(fixture.work().join("new")).unwrap(), b"keep");
}

#[test]
fn changed_inspection_token_and_cancel_leave_live_bytes() {
    let fixture = Fixture::new();
    let file = fixture.work().join("note");
    fs::write(&file, b"before").unwrap();
    let store = fixture.store();
    let reference = captured(&store, &["note"]);
    fs::write(&file, b"one").unwrap();
    let inspected = store
        .inspect(&reference, None, &Cancellation::default())
        .unwrap();
    fs::write(&file, b"two").unwrap();
    let report = store
        .restore(
            &reference,
            None,
            &inspected.expected,
            false,
            &Cancellation::default(),
        )
        .unwrap();
    assert!(!report.conflicts.is_empty());
    assert!(report.undo.is_none());
    assert_eq!(fs::read(&file).unwrap(), b"two");
    let cancelled = Cancellation::default();
    cancelled.cancel();
    assert!(
        store
            .restore(&reference, None, "latest", false, &cancelled)
            .is_err()
    );
    assert_eq!(fs::read(file).unwrap(), b"two");
}

#[test]
fn change_between_inspection_and_undo_capture_is_a_conflict() {
    let fixture = Fixture::new();
    let file = fixture.work().join("note");
    fs::write(&file, b"before").unwrap();
    let store = fixture.store();
    let reference = captured(&store, &["note"]);
    fs::write(&file, b"acknowledged").unwrap();
    let inspected = store
        .inspect(&reference, None, &Cancellation::default())
        .unwrap();
    let changed = file.clone();
    BEFORE_UNDO_CAPTURE.with(|hook| {
        *hook.borrow_mut() = Some(Box::new(move || {
            fs::write(&changed, b"later writer").unwrap()
        }));
    });
    let report = store
        .restore(
            &reference,
            None,
            &inspected.expected,
            false,
            &Cancellation::default(),
        )
        .unwrap();
    assert_eq!(report.conflicts.len(), 1);
    assert!(report.restored.is_empty());
    assert!(report.undo.is_none());
    assert_eq!(fs::read(file).unwrap(), b"later writer");
}

#[test]
fn write_after_install_conflicts_with_emitted_undo_token() {
    let fixture = Fixture::new();
    let file = fixture.work().join("note");
    fs::write(&file, b"before").unwrap();
    let store = fixture.store();
    let reference = captured(&store, &["note"]);
    fs::write(&file, b"after").unwrap();
    let changed = file.clone();
    AFTER_INSTALL.with(|hook| {
        *hook.borrow_mut() = Some(Box::new(move || {
            fs::write(&changed, b"outside writer").unwrap()
        }));
    });
    let report = store
        .restore(&reference, None, "latest", false, &Cancellation::default())
        .unwrap();
    let undo = report.undo.unwrap();
    let undone = store
        .restore(
            &undo.reference,
            Some(&undo.paths),
            &undo.expected,
            false,
            &Cancellation::default(),
        )
        .unwrap();
    assert_eq!(undone.conflicts.len(), 1);
    assert!(undone.restored.is_empty());
    assert_eq!(fs::read(file).unwrap(), b"outside writer");
}

#[test]
fn partial_installation_reports_precise_undo() {
    let fixture = Fixture::new();
    let a = fixture.work().join("a");
    let b = fixture.work().join("b");
    fs::write(&a, b"a-old").unwrap();
    fs::write(&b, b"b-old").unwrap();
    let store = fixture.store();
    let reference = captured(&store, &["a", "b"]);
    fs::write(&a, b"a-new").unwrap();
    fs::remove_file(&b).unwrap();
    fs::create_dir(&b).unwrap();
    fs::write(b.join("child"), b"must survive").unwrap();
    let report = store
        .restore(&reference, None, "latest", false, &Cancellation::default())
        .unwrap();
    assert_eq!(report.restored, [("a".into(), Action::Replace)]);
    assert_eq!(report.failed.len(), 1);
    assert_eq!(report.undo.as_ref().unwrap().paths, ["a"]);
    assert_eq!(fs::read(&a).unwrap(), b"a-old");
    assert_eq!(fs::read(b.join("child")).unwrap(), b"must survive");
}

#[test]
fn corrupt_saved_file_never_removes_live_entry() {
    use std::io::{Seek, SeekFrom, Write};
    let fixture = Fixture::new();
    let file = fixture.work().join("note");
    fs::write(&file, b"before").unwrap();
    let store = fixture.store();
    let reference = captured(&store, &["note"]);
    let loaded = store.load(&reference, &Cancellation::default()).unwrap();
    let State::File { offset, .. } = &loaded.snapshot.entries["note"] else {
        panic!("expected file")
    };
    let offset = *offset;
    drop(loaded);
    fs::write(&file, b"after").unwrap();
    let mut pack = crate::disk::private_options()
        .write(true)
        .open(store.directory.join(format!("{reference}.rcp")))
        .unwrap();
    pack.seek(SeekFrom::Start(offset)).unwrap();
    pack.write_all(b"X").unwrap();
    drop(pack);
    assert!(
        store
            .restore(&reference, None, "latest", false, &Cancellation::default())
            .is_err()
    );
    assert_eq!(fs::read(file).unwrap(), b"after");
}

#[test]
fn directory_replaces_file_and_precise_undo_replaces_directory() {
    let fixture = Fixture::new();
    let directory = fixture.work().join("d");
    fs::create_dir(&directory).unwrap();
    fs::write(directory.join("child"), b"saved child").unwrap();
    let store = fixture.store();
    let reference = captured(&store, &["d"]);
    fs::remove_file(directory.join("child")).unwrap();
    fs::remove_dir(&directory).unwrap();
    fs::write(&directory, b"live file").unwrap();
    let report = store
        .restore(&reference, None, "latest", false, &Cancellation::default())
        .unwrap();
    assert!(report.failed.is_empty(), "{:?}", report.failed);
    assert_eq!(fs::read(directory.join("child")).unwrap(), b"saved child");
    let undo = report.undo.unwrap();
    assert_eq!(undo.paths, ["d", "d/child"]);
    let undone = store
        .restore(
            &undo.reference,
            Some(&undo.paths),
            &undo.expected,
            false,
            &Cancellation::default(),
        )
        .unwrap();
    assert!(undone.failed.is_empty(), "{:?}", undone.failed);
    assert_eq!(fs::read(directory).unwrap(), b"live file");
}

#[test]
fn multi_megabyte_restore_streams_exact_bytes() {
    let fixture = Fixture::new();
    let file = fixture.work().join("large.bin");
    let saved: Vec<u8> = (0..5 * 1024 * 1024)
        .map(|index| (index * 37) as u8)
        .collect();
    fs::write(&file, &saved).unwrap();
    let store = fixture.store();
    let reference = captured(&store, &["large.bin"]);
    fs::write(&file, vec![9; saved.len()]).unwrap();
    let report = store
        .restore(&reference, None, "latest", false, &Cancellation::default())
        .unwrap();
    assert!(report.failed.is_empty());
    assert_eq!(fs::read(file).unwrap(), saved);
}

#[test]
fn case_only_rename_restores_once_and_undoes() {
    let fixture = Fixture::new();
    let original = fixture.work().join("Case.txt");
    let renamed = fixture.work().join("case.txt");
    fs::write(&original, b"before").unwrap();
    let store = fixture.store();
    let reference = captured(&store, &["."]);
    fs::rename(&original, &renamed).unwrap();
    let case_insensitive = fs::symlink_metadata(&original).is_ok();
    fs::write(&renamed, b"changed").unwrap();
    let inspected = store
        .inspect(&reference, None, &Cancellation::default())
        .unwrap();
    let report = store
        .restore(
            &reference,
            None,
            &inspected.expected,
            false,
            &Cancellation::default(),
        )
        .unwrap();
    assert!(report.failed.is_empty(), "{:?}", report.failed);
    assert_eq!(fs::read(&original).unwrap(), b"before");
    if case_insensitive {
        assert_eq!(report.restored.len(), 1, "case aliases must name one entry");
    } else {
        assert_eq!(
            report.restored.len(),
            2,
            "case-sensitive entries are distinct"
        );
    }
    let undo = report.undo.unwrap();
    let undone = store
        .restore(
            &undo.reference,
            Some(&undo.paths),
            &undo.expected,
            false,
            &Cancellation::default(),
        )
        .unwrap();
    assert!(undone.failed.is_empty(), "{:?}", undone.failed);
    assert_eq!(fs::read(&renamed).unwrap(), b"changed");
    if !case_insensitive {
        assert!(!original.exists());
    }
}

#[cfg(unix)]
#[test]
fn undo_directory_mode_does_not_capture_or_restore_unchanged_children() {
    use std::os::unix::fs::PermissionsExt;
    let fixture = Fixture::new();
    let directory = fixture.work().join("d");
    fs::create_dir(&directory).unwrap();
    let child = directory.join("a");
    fs::write(&child, b"original child").unwrap();
    fs::set_permissions(&directory, fs::Permissions::from_mode(0o700)).unwrap();
    let store = fixture.store();
    let reference = captured(&store, &["d"]);
    fs::set_permissions(&directory, fs::Permissions::from_mode(0o755)).unwrap();
    let report = store
        .restore(&reference, None, "latest", false, &Cancellation::default())
        .unwrap();
    assert!(report.failed.is_empty(), "{:?}", report.failed);
    let undo = report.undo.unwrap();
    assert_eq!(undo.paths, ["d"]);
    let undo_pack = store
        .load(&undo.reference, &Cancellation::default())
        .unwrap();
    assert_eq!(undo_pack.snapshot.entries.len(), 1);
    assert!(undo_pack.snapshot.roots.is_empty());
    drop(undo_pack);
    fs::write(&child, b"outside edit").unwrap();
    let undone = store
        .restore(
            &undo.reference,
            Some(&undo.paths),
            &undo.expected,
            false,
            &Cancellation::default(),
        )
        .unwrap();
    assert!(undone.conflicts.is_empty(), "{:?}", undone.conflicts);
    assert!(undone.failed.is_empty(), "{:?}", undone.failed);
    assert_eq!(fs::read(&child).unwrap(), b"outside edit");
    assert_eq!(
        fs::metadata(&directory).unwrap().permissions().mode() & 0o777,
        0o755
    );
}

#[cfg(unix)]
#[test]
fn restrictive_directory_mode_is_applied_after_children() {
    use std::os::unix::fs::PermissionsExt;
    let fixture = Fixture::new();
    let directory = fixture.work().join("locked");
    fs::create_dir(&directory).unwrap();
    fs::write(directory.join("child"), b"saved").unwrap();
    files::Mode(0o500).apply_path(&directory).unwrap();
    let store = fixture.store();
    let reference = captured(&store, &["locked"]);
    files::Mode(0o700).apply_path(&directory).unwrap();
    fs::remove_file(directory.join("child")).unwrap();
    fs::remove_dir(&directory).unwrap();
    let report = store
        .restore(&reference, None, "latest", false, &Cancellation::default())
        .unwrap();
    assert!(report.failed.is_empty(), "{:?}", report.failed);
    assert_eq!(fs::read(directory.join("child")).unwrap(), b"saved");
    assert_eq!(
        fs::metadata(&directory).unwrap().permissions().mode() & 0o777,
        0o500
    );
    files::Mode(0o700).apply_path(&directory).unwrap();
}

#[cfg(unix)]
#[test]
fn link_entry_restores_without_following_target() {
    let fixture = Fixture::new();
    fs::write(fixture.work().join("target"), b"target").unwrap();
    std::os::unix::fs::symlink("target", fixture.work().join("link")).unwrap();
    let store = fixture.store();
    let reference = captured(&store, &["link"]);
    fs::remove_file(fixture.work().join("link")).unwrap();
    fs::write(fixture.work().join("link"), b"changed").unwrap();
    let report = store
        .restore(&reference, None, "latest", false, &Cancellation::default())
        .unwrap();
    assert!(report.failed.is_empty());
    assert_eq!(
        fs::read_link(fixture.work().join("link")).unwrap(),
        std::path::Path::new("target")
    );
    assert_eq!(fs::read(fixture.work().join("target")).unwrap(), b"target");
}

#[cfg(windows)]
#[test]
fn broken_directory_link_keeps_its_kind() {
    use std::os::windows::fs::FileTypeExt;
    let fixture = Fixture::new();
    let link = fixture.work().join("link");
    if std::os::windows::fs::symlink_dir("missing-directory", &link).is_err() {
        return;
    }
    let store = fixture.store();
    let reference = captured(&store, &["link"]);
    fs::remove_dir(&link).unwrap();
    fs::write(&link, b"live").unwrap();
    let report = store
        .restore(&reference, None, "latest", false, &Cancellation::default())
        .unwrap();
    assert!(report.failed.is_empty(), "{:?}", report.failed);
    assert!(
        fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink_dir()
    );
    assert_eq!(
        fs::read_link(&link).unwrap(),
        std::path::Path::new("missing-directory")
    );
}
