use super::*;
use crate::test_support::Directory;
use std::io::{Read, Seek, SeekFrom, Write};
use std::sync::atomic::Ordering;

struct Fixture(Directory);
impl Fixture {
    fn new() -> Self {
        let directory = Directory::new();
        fs::create_dir(directory.path().join("workspace")).unwrap();
        fs::create_dir(directory.path().join("home")).unwrap();
        Self(directory)
    }
    fn workspace(&self) -> PathBuf {
        self.0.path().join("workspace")
    }
    fn store(&self) -> Arc<Store> {
        Store::open(&self.0.path().join("home"), &self.workspace()).unwrap()
    }
}

#[test]
fn published_pack_survives_restart_and_expires_from_creation() {
    let fixture = Fixture::new();
    fs::write(fixture.workspace().join("hello.txt"), b"before").unwrap();
    let store = fixture.store();
    let cancel = Cancellation::default();
    let first = store
        .capture("write", "test", &["hello.txt".into()], &cancel)
        .unwrap();
    let second = store
        .capture("write", "test", &["hello.txt".into()], &cancel)
        .unwrap();
    assert_ne!(first.reference, second.reference);
    // Both captures may share a millisecond, so keep both packs leased while
    // checking that cleanup cannot remove an active reader's pack.
    let _second_reader = store
        .load(second.reference.as_ref().unwrap(), &cancel)
        .unwrap();
    let reference = first.reference.unwrap();
    let loaded = store.load(&reference, &cancel).unwrap();
    let state = &loaded.snapshot.entries["hello.txt"];
    assert_eq!(
        loaded.snapshot.expires(),
        loaded.snapshot.created + TTL_MILLIS
    );
    let mut bytes = Vec::new();
    loaded
        .reader(state)
        .unwrap()
        .read_to_end(&mut bytes)
        .unwrap();
    assert_eq!(bytes, b"before");
    drop(loaded);
    drop(store);
    let restarted = fixture.store();
    let loaded = restarted.load(&reference, &cancel).unwrap();
    loaded
        .verify(&loaded.snapshot.entries["hello.txt"], &cancel)
        .unwrap();
    let expires = loaded.snapshot.expires();
    assert_eq!(restarted.cleanup(expires - 1).unwrap(), 0);
    assert_eq!(
        restarted.cleanup(expires).unwrap(),
        0,
        "an active reader holds its pack"
    );
    drop(loaded);
    assert_eq!(restarted.cleanup(expires).unwrap(), 1);
    assert!(restarted.load(&reference, &cancel).is_err());
}

#[test]
fn listing_applies_limit_after_skipping_corrupt_newer_and_expired_packs() {
    let fixture = Fixture::new();
    fs::write(fixture.workspace().join("data"), b"before").unwrap();
    let store = fixture.store();
    let cancel = Cancellation::default();
    let older = store
        .capture("write", "test", &["data".into()], &cancel)
        .unwrap()
        .reference
        .unwrap();
    let newer = store
        .capture("edit", "test", &["data".into()], &cancel)
        .unwrap()
        .reference
        .unwrap();
    let mut file = files::options()
        .read(true)
        .write(true)
        .open(store.directory.join(format!("{newer}.rcp")))
        .unwrap();
    file.seek(SeekFrom::End(-25)).unwrap();
    file.write_all(&[1]).unwrap();
    drop(file);
    fs::write(
        store
            .directory
            .join("rc-0000000000000001-00000001-0000000000000001.rcp"),
        b"expired",
    )
    .unwrap();
    let listed = store.list(1, &cancel).unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].reference, older);
}

#[test]
fn payload_and_metadata_corruption_are_distinguished() {
    let fixture = Fixture::new();
    fs::write(fixture.workspace().join("data"), b"abcdefgh").unwrap();
    let store = fixture.store();
    let cancel = Cancellation::default();
    let reference = store
        .capture("write", "test", &["data".into()], &cancel)
        .unwrap()
        .reference
        .unwrap();
    let pack = store.directory.join(format!("{reference}.rcp"));
    let loaded = store.load(&reference, &cancel).unwrap();
    let state = loaded.snapshot.entries["data"].clone();
    let State::File { offset, .. } = state else {
        panic!("expected file")
    };
    drop(loaded);
    let mut file = files::options().read(true).write(true).open(&pack).unwrap();
    file.seek(SeekFrom::Start(offset)).unwrap();
    file.write_all(b"Z").unwrap();
    drop(file);
    let loaded = store.load(&reference, &cancel).unwrap();
    assert!(
        loaded
            .verify(&state, &cancel)
            .unwrap_err()
            .contains("Corrupt recovery payload")
    );
    drop(loaded);
    let mut file = files::options().read(true).write(true).open(&pack).unwrap();
    file.seek(SeekFrom::End(-24)).unwrap();
    file.write_all(&u64::MAX.to_le_bytes()).unwrap();
    drop(file);
    assert!(store.load(&reference, &cancel).is_err());
    assert_eq!(
        store.cleanup(u64::MAX).unwrap(),
        1,
        "corrupt packs obey the same TTL"
    );
}

#[test]
fn changed_metadata_fails_checksum_before_decoding() {
    let fixture = Fixture::new();
    fs::write(fixture.workspace().join("data"), b"content").unwrap();
    let store = fixture.store();
    let cancel = Cancellation::default();
    let reference = store
        .capture("write", "test", &["data".into()], &cancel)
        .unwrap()
        .reference
        .unwrap();
    let pack = store.directory.join(format!("{reference}.rcp"));
    let mut file = files::options().read(true).write(true).open(&pack).unwrap();
    file.seek(SeekFrom::End(-25)).unwrap();
    file.write_all(&[1]).unwrap();
    drop(file);
    assert!(
        store
            .load(&reference, &cancel)
            .err()
            .unwrap()
            .contains("Corrupt recovery metadata")
    );
}

#[test]
fn format_preserves_directory_link_kind_and_rejects_older_version() {
    let fixture = Fixture::new();
    let pack = fixture.0.path().join("link.rcp");
    let mut file = files::options()
        .read(true)
        .write(true)
        .create_new(true)
        .open(&pack)
        .unwrap();
    let mut entries = std::collections::BTreeMap::new();
    entries.insert(
        "broken-link".into(),
        State::Link {
            target: "missing".into(),
            directory: true,
        },
    );
    let snapshot = Snapshot {
        reference: "rc-0000000000000001-00000001-0000000000000001".into(),
        created: 1,
        tool: "write".into(),
        origin: "test".into(),
        entries,
        roots: Vec::new(),
        excluded: Vec::new(),
        partial: Vec::new(),
    };
    format::header(&mut file, snapshot.created).unwrap();
    format::finish(&mut file, &format::encode(&snapshot).unwrap()).unwrap();
    let decoded = format::read(&mut file).unwrap();
    assert_eq!(
        decoded.entries["broken-link"],
        snapshot.entries["broken-link"]
    );
    file.seek(SeekFrom::Start(0)).unwrap();
    file.write_all(b"JECRCP01").unwrap();
    assert!(format::read(&mut file).is_err());
}

#[test]
fn cancelled_capture_leaves_no_published_pack() {
    let fixture = Fixture::new();
    fs::write(fixture.workspace().join("large"), vec![7u8; 2 << 20]).unwrap();
    let store = fixture.store();
    let cancel = Cancellation::default();
    cancel.cancel();
    assert!(
        store
            .capture("write", "test", &["large".into()], &cancel)
            .is_err()
    );
    assert!(store.list(10, &Cancellation::default()).unwrap().is_empty());
}

#[test]
fn cancellation_during_streaming_discards_unpublished_bytes() {
    let fixture = Fixture::new();
    fs::write(fixture.workspace().join("large"), vec![7u8; 2 << 20]).unwrap();
    let store = fixture.store();
    store.cancel_after_chunk.store(true, Ordering::Relaxed);
    let cancel = Cancellation::default();
    assert_eq!(
        store
            .capture("write", "test", &["large".into()], &cancel)
            .err()
            .unwrap(),
        "Operation cancelled"
    );
    assert!(cancel.requested());
    assert!(store.list(10, &Cancellation::default()).unwrap().is_empty());
    assert!(fs::read_dir(&store.directory).unwrap().all(|entry| {
        !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .ends_with(".pending")
    }));
}

#[test]
fn absent_explicit_path_records_an_empty_scope_without_guessing_its_type() {
    let fixture = Fixture::new();
    let store = fixture.store();
    let cancel = Cancellation::default();
    let captured = store
        .capture("write", "test", &["new.txt".into()], &cancel)
        .unwrap();
    assert_eq!(captured.saved, 1);
    let loaded = store
        .load(captured.reference.as_deref().unwrap(), &cancel)
        .unwrap();
    assert_eq!(loaded.snapshot.entries.get("new.txt"), Some(&State::Absent));
    assert_eq!(loaded.snapshot.roots, ["new.txt"]);
}

#[test]
fn exact_undo_capture_does_not_expand_directory_or_claim_membership() {
    let fixture = Fixture::new();
    fs::create_dir(fixture.workspace().join("d")).unwrap();
    fs::write(fixture.workspace().join("d").join("untouched"), b"keep").unwrap();
    let store = fixture.store();
    let cancel = Cancellation::default();
    let captured = store
        .capture_exact("undo restore", "test", &["d".into()], &cancel)
        .unwrap();
    let loaded = store
        .load(captured.reference.as_deref().unwrap(), &cancel)
        .unwrap();
    assert_eq!(loaded.snapshot.entries.len(), 1);
    assert!(matches!(
        loaded.snapshot.entries.get("d"),
        Some(State::Directory { .. })
    ));
    assert!(loaded.snapshot.roots.is_empty());
}

#[test]
fn exact_undo_capture_synthesizes_absent_below_saved_file() {
    let fixture = Fixture::new();
    fs::write(fixture.workspace().join("d"), b"parent file").unwrap();
    let store = fixture.store();
    let cancel = Cancellation::default();
    let captured = store
        .capture_exact(
            "undo restore",
            "test",
            &["d".into(), "d/child".into()],
            &cancel,
        )
        .unwrap();
    let loaded = store
        .load(captured.reference.as_deref().unwrap(), &cancel)
        .unwrap();
    assert!(matches!(
        loaded.snapshot.entries.get("d"),
        Some(State::File { .. })
    ));
    assert_eq!(loaded.snapshot.entries.get("d/child"), Some(&State::Absent));
    assert!(loaded.snapshot.partial.is_empty());
    assert!(loaded.snapshot.roots.is_empty());
}

#[test]
fn large_file_reader_is_bounded_and_seekable() {
    let fixture = Fixture::new();
    let bytes: Vec<u8> = (0..3 * 1024 * 1024).map(|n| n as u8).collect();
    fs::write(fixture.workspace().join("large"), &bytes).unwrap();
    let store = fixture.store();
    let cancel = Cancellation::default();
    let reference = store
        .capture("write", "test", &["large".into()], &cancel)
        .unwrap()
        .reference
        .unwrap();
    let loaded = store.load(&reference, &cancel).unwrap();
    let state = &loaded.snapshot.entries["large"];
    let mut reader = loaded.reader(state).unwrap();
    let mut another = loaded.reader(state).unwrap();
    assert_eq!(
        reader.seek(SeekFrom::End(-10)).unwrap(),
        bytes.len() as u64 - 10
    );
    let mut tail = Vec::new();
    reader.read_to_end(&mut tail).unwrap();
    assert_eq!(tail, bytes[bytes.len() - 10..]);
    let mut first = [0u8; 4];
    another.read_exact(&mut first).unwrap();
    assert_eq!(first, bytes[..4]);
    assert!(reader.seek(SeekFrom::End(1)).is_err());
    loaded.verify(state, &cancel).unwrap();
}

#[test]
fn empty_directory_scope_still_records_absence_proof() {
    let fixture = Fixture::new();
    let store = fixture.store();
    let cancel = Cancellation::default();
    let captured = store
        .capture("bash", "test", &[String::new()], &cancel)
        .unwrap();
    let loaded = store
        .load(captured.reference.as_deref().unwrap(), &cancel)
        .unwrap();
    assert!(loaded.snapshot.roots.contains(&String::new()));
    assert_eq!(captured.saved, 0);
}

#[test]
fn cleanup_removes_only_expired_recognized_unpublished_files() {
    let fixture = Fixture::new();
    let store = fixture.store();
    let pending = store
        .directory
        .join(".rc-0000000000000001-00000001-0000000000000001.pending");
    let foreign = store.directory.join(".foreign.pending");
    fs::write(&pending, b"incomplete pack").unwrap();
    fs::write(&foreign, b"foreign data").unwrap();
    assert_eq!(store.cleanup(TTL_MILLIS).unwrap(), 0);
    assert_eq!(store.cleanup(TTL_MILLIS + 1).unwrap(), 1);
    assert!(!pending.exists());
    assert!(foreign.exists());
}

#[test]
fn publication_collision_preserves_existing_pack() {
    let fixture = Fixture::new();
    let store = fixture.store();
    let created = super::super::clock::now();
    let reference = format!("rc-{created:016x}-00000001-0000000000000001");
    let final_path = store.directory.join(format!("{reference}.rcp"));
    fs::write(&final_path, b"existing").unwrap();
    assert!(store.publish(&reference, &Cancellation::default()).is_err());
    assert_eq!(fs::read(&final_path).unwrap(), b"existing");
}

#[test]
fn explicit_internal_storage_is_never_captured() {
    let fixture = Fixture::new();
    let home = fixture.workspace().join(".jecode");
    let store = Store::open(&home, &fixture.workspace()).unwrap();
    let excluded = store.excluded.first().unwrap().clone();
    let captured = store
        .capture("write", "test", &[excluded], &Cancellation::default())
        .unwrap();
    assert!(captured.reference.is_none());
    assert!(!captured.partial.is_empty());
}

#[test]
fn broad_scope_excludes_all_checkpoint_and_retired_archives_under_home() {
    let fixture = Fixture::new();
    let home = fixture.workspace().join(".jecode");
    let store = Store::open(&home, &fixture.workspace()).unwrap();
    let other = home.join("recovery").join("another-workspace");
    fs::create_dir_all(&other).unwrap();
    fs::write(other.join("other.rcp"), b"other project").unwrap();
    let legacy = home.join("checkpoints");
    fs::write(legacy.join("old.bin"), b"old archive").unwrap();
    fs::write(fixture.workspace().join("source.txt"), b"source").unwrap();
    let cancel = Cancellation::default();
    let captured = store
        .capture("bash", "test", &[String::new()], &cancel)
        .unwrap();
    let loaded = store
        .load(captured.reference.as_deref().unwrap(), &cancel)
        .unwrap();
    assert!(loaded.snapshot.entries.contains_key("source.txt"));
    assert!(!loaded.snapshot.entries.keys().any(
        |path| path.starts_with(".jecode/recovery") || path.starts_with(".jecode/checkpoints")
    ));
    for path in [
        ".jecode/recovery/another-workspace/other.rcp",
        ".jecode/checkpoints/old.bin",
    ] {
        let blocked = store
            .capture("write", "test", &[path.into()], &cancel)
            .unwrap();
        assert!(blocked.reference.is_none());
        assert!(!blocked.partial.is_empty());
    }
}

#[test]
fn home_sweep_expires_inactive_workspace_without_touching_foreign_data() {
    let fixture = Fixture::new();
    fs::write(fixture.workspace().join("data"), b"old").unwrap();
    let store = fixture.store();
    let cancel = Cancellation::default();
    let reference = store
        .capture("write", "test", &["data".into()], &cancel)
        .unwrap()
        .reference
        .unwrap();
    let loaded = store.load(&reference, &cancel).unwrap();
    let expiry = loaded.snapshot.expires();
    let pack = store.directory.join(format!("{reference}.rcp"));
    let foreign = fixture
        .0
        .path()
        .join("home")
        .join("checkpoints")
        .join("foreign-00000000000000000000000000000000");
    fs::create_dir(&foreign).unwrap();
    fs::write(foreign.join("workspace.id"), b"not that digest").unwrap();
    fs::write(foreign.join(format!("{reference}.rcp")), b"foreign").unwrap();
    drop(loaded);
    drop(store);
    assert_eq!(
        Store::sweep_home(&fixture.0.path().join("home"), expiry, &cancel).unwrap(),
        1
    );
    assert!(!pack.exists());
    assert!(foreign.join(format!("{reference}.rcp")).exists());
}

#[path = "process_tests.rs"]
mod process_tests;

#[path = "layout_tests.rs"]
mod layout_tests;

#[path = "reader_tests.rs"]
mod reader_tests;
