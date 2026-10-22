use super::*;
use crate::checkpoints::identity;
use crate::checkpoints::record::{Prepare, Record, State};
use crate::checkpoints::store::Store;
use crate::test_support::Directory;
use std::time::Duration;

fn fixture() -> (Directory, Location) {
    let fixture = Directory::new();
    let workspace = fixture.path().join("project");
    fs::create_dir(&workspace).unwrap();
    let location = identity::locate(&fixture.path().join("home"), &workspace, true)
        .unwrap()
        .unwrap();
    (fixture, location)
}

fn age(archive: &Path) {
    set_age(archive, Duration::from_secs(8 * 24 * 60 * 60));
}

fn set_age(archive: &Path, elapsed: Duration) {
    let old = crate::sessions::now() - elapsed.as_millis() as u64;
    files::atomic_write(&last_use_path(archive), old.to_string().as_bytes()).unwrap();
}

fn legacy_location(fixture: &Directory, location: &Location) -> Location {
    let hash = location
        .directory
        .file_name()
        .unwrap()
        .to_string_lossy()
        .strip_prefix("project-")
        .unwrap()
        .to_string();
    let legacy = fixture.path().join("home/checkpoints").join(hash);
    fs::remove_file(&legacy).unwrap();
    fs::create_dir(&legacy).unwrap();
    let text = fs::read_to_string(location.directory.join("workspace.json")).unwrap();
    fs::write(
        legacy.join("workspace.json"),
        text.replace("\"version\": 2", "\"version\": 1"),
    )
    .unwrap();
    Location {
        root: location.root.clone(),
        directory: legacy,
    }
}

fn legacy_with_object() -> (Directory, Location, u64, Vec<u8>) {
    let (fixture, location) = fixture();
    let bytes = b"exact legacy checkpoint bytes\0with a binary tail\xff".to_vec();
    let mut modern = Store::open(location.clone()).unwrap();
    let id = modern
        .transaction(true, |tx| tx.object(bytes.clone().into(), None))
        .unwrap();
    drop(modern);
    let legacy = legacy_location(&fixture, &location);
    fs::rename(
        layout::generation_directory(&location.directory, 1),
        legacy.directory.join("gen-000001"),
    )
    .unwrap();
    let current = layout::current(&location.directory);
    if current.exists() {
        fs::rename(current, legacy.directory.join("CURRENT")).unwrap();
    }
    fs::copy(
        location.directory.join("policy.json"),
        legacy.directory.join("policy.json"),
    )
    .unwrap();
    (fixture, legacy, id, bytes)
}

#[test]
fn new_layout_expires_idle_history_and_keeps_sequence_identity_and_policy() {
    let (_fixture, location) = fixture();
    assert_eq!(layout::version(&location.directory), Some(2));
    let mut store = Store::open(location.clone()).unwrap();
    let first = store
        .transaction(true, |tx| Ok(tx.point("test", "first", "test", false)))
        .unwrap();
    drop(store);
    age(&location.directory);
    let archives = location.directory.parent().unwrap();
    assert_eq!(sweep(archives).unwrap(), 1);
    assert!(!layout::history(&location.directory).exists());
    assert!(location.directory.join("workspace.json").exists());
    assert!(location.directory.join("policy.json").exists());
    assert!(high_water(&location.directory).unwrap() > first);
    let mut reopened = Store::open(location.clone()).unwrap();
    let second = reopened
        .transaction(true, |tx| Ok(tx.point("test", "second", "test", false)))
        .unwrap();
    assert!(second > first);
    assert!(!reopened.index.retained(first));
    touch(&location.directory).unwrap();
    drop(reopened);
    let reopened = Store::open(location.clone()).unwrap();
    assert!(!reopened.index.retained(first));
    drop(reopened);
    let _ = crate::checkpoints::gc::collect(&location, true).unwrap();
    let after_collection = Store::open(location.clone()).unwrap();
    assert!(!after_collection.index.retained(first));
    assert_eq!(
        identity::locate(archives.parent().unwrap(), &location.root, false)
            .unwrap()
            .unwrap()
            .directory,
        location.directory
    );
}

#[test]
fn retention_uses_real_days_and_hourly_throttles_use_real_hours() {
    assert_eq!(
        RETENTION_MILLIS,
        Duration::from_secs(7 * 24 * 60 * 60).as_millis() as u64
    );
    assert_eq!(
        SWEEP_MILLIS,
        Duration::from_secs(60 * 60).as_millis() as u64
    );

    let (_fixture, location) = fixture();
    let archive = &location.directory;
    let archives = archive.parent().unwrap();
    let store = Store::open(location.clone()).unwrap();
    drop(store);
    for elapsed in [
        Duration::from_secs(24 * 60 * 60),
        Duration::from_secs(6 * 24 * 60 * 60 + 23 * 60 * 60 + 59 * 60),
    ] {
        set_age(archive, elapsed);
        assert_eq!(sweep(archives).unwrap(), 0);
        assert!(layout::history(archive).exists());
    }

    set_age(archive, Duration::from_secs(2 * 60 * 60));
    let stale_activity = fs::read(last_use_path(archive)).unwrap();
    let now = crate::sessions::now();
    let touches = LAST_TOUCH.get_or_init(|| Mutex::new(HashMap::new()));
    touches.lock().unwrap().insert(
        archive.to_path_buf(),
        now - Duration::from_secs(30 * 60).as_millis() as u64,
    );
    touch_throttled(archive).unwrap();
    assert_eq!(fs::read(last_use_path(archive)).unwrap(), stale_activity);
    touches.lock().unwrap().insert(
        archive.to_path_buf(),
        now - Duration::from_secs(61 * 60).as_millis() as u64,
    );
    touch_throttled(archive).unwrap();
    assert_ne!(fs::read(last_use_path(archive)).unwrap(), stale_activity);

    age(archive);
    let sweeps = LAST_SWEEP.get_or_init(|| Mutex::new(HashMap::new()));
    sweeps.lock().unwrap().insert(
        archives.to_path_buf(),
        now - Duration::from_secs(30 * 60).as_millis() as u64,
    );
    assert_eq!(sweep_throttled(archives).unwrap(), 0);
    assert!(layout::history(archive).exists());
    sweeps.lock().unwrap().insert(
        archives.to_path_buf(),
        now - Duration::from_secs(61 * 60).as_millis() as u64,
    );
    assert_eq!(sweep_throttled(archives).unwrap(), 1);
}

#[test]
fn failed_expiry_blocks_reopen_and_retries_without_renewing_activity() {
    let (_fixture, location) = fixture();
    let mut store = Store::open(location.clone()).unwrap();
    let old = store
        .transaction(true, |tx| Ok(tx.point("test", "old", "test", false)))
        .unwrap();
    drop(store);
    age(&location.directory);
    let before = fs::read(last_use_path(&location.directory)).unwrap();
    let obstruction = location.directory.join("history.expired");
    fs::write(&obstruction, b"obstructed detach").unwrap();

    assert_eq!(sweep(location.directory.parent().unwrap()).unwrap(), 0);
    assert!(expiry_pending_path(&location.directory).exists());
    assert!(Store::open(location.clone()).is_err());
    assert_eq!(
        fs::read(last_use_path(&location.directory)).unwrap(),
        before
    );
    assert!(layout::history(&location.directory).exists());

    fs::remove_file(obstruction).unwrap();
    let mut reopened = Store::open(location.clone()).unwrap();
    assert!(!expiry_pending_path(&location.directory).exists());
    assert!(!reopened.index.retained(old));
    let new = reopened
        .transaction(true, |tx| Ok(tx.point("test", "new", "test", false)))
        .unwrap();
    assert!(new > old);
}

#[test]
fn detached_expiry_resumes_before_reopening_history() {
    let (_fixture, location) = fixture();
    let mut store = Store::open(location.clone()).unwrap();
    let old = store
        .transaction(true, |tx| Ok(tx.point("test", "old", "test", false)))
        .unwrap();
    drop(store);
    age(&location.directory);
    let obstruction = location.directory.join("history.expired");
    fs::write(&obstruction, b"obstructed detach").unwrap();
    assert_eq!(sweep(location.directory.parent().unwrap()).unwrap(), 0);
    fs::remove_file(&obstruction).unwrap();
    fs::rename(layout::history(&location.directory), &obstruction).unwrap();

    let mut reopened = Store::open(location.clone()).unwrap();
    assert!(!obstruction.exists());
    assert!(!expiry_pending_path(&location.directory).exists());
    assert!(!reopened.index.retained(old));
    let new = reopened
        .transaction(true, |tx| Ok(tx.point("test", "new", "test", false)))
        .unwrap();
    assert!(new > old);
}

#[test]
fn damaged_expiry_marker_cannot_authorize_history_removal() {
    let (_fixture, location) = fixture();
    let store = Store::open(location.clone()).unwrap();
    drop(store);
    fs::write(expiry_pending_path(&location.directory), b"damaged").unwrap();

    assert!(Store::open(location.clone()).is_err());
    assert!(layout::history(&location.directory).exists());
}

#[test]
fn live_reader_and_pinned_or_pending_history_never_expire() {
    let (_fixture, location) = fixture();
    let mut store = Store::open(location.clone()).unwrap();
    let pinned = store
        .transaction(true, |tx| Ok(tx.point("test", "pin", "test", true)))
        .unwrap();
    age(&location.directory);
    assert_eq!(sweep(location.directory.parent().unwrap()).unwrap(), 0);
    drop(store);
    assert!(
        read_number(&last_use_path(&location.directory))
            .unwrap()
            .unwrap()
            <= crate::sessions::now()
                - Duration::from_secs(7 * 24 * 60 * 60 + 60 * 60).as_millis() as u64
    );
    assert_eq!(sweep(location.directory.parent().unwrap()).unwrap(), 0);
    let mut store = Store::open(location.clone()).unwrap();
    store
        .transaction(true, |tx| {
            let seq = tx.seq();
            tx.record(Record::Pin {
                seq,
                target: pinned,
                pinned: false,
                label: String::new(),
            });
            let seq = tx.seq();
            tx.record(Record::Prepare(Prepare {
                seq,
                owner: "test".into(),
                origin: "test".into(),
                kind: "edit".into(),
                path: "file.txt".into(),
                before: State::Absent,
                after: State::Absent,
                time: crate::sessions::now(),
            }));
            Ok(())
        })
        .unwrap();
    drop(store);
    assert_eq!(sweep(location.directory.parent().unwrap()).unwrap(), 0);
    assert!(layout::history(&location.directory).exists());
}

#[test]
fn unknown_legacy_activity_gets_full_grace_period() {
    let (fixture, location) = fixture();
    let legacy = legacy_location(&fixture, &location);
    assert_eq!(layout::version(&legacy.directory), Some(1));
    let _journal = Journal::open(&legacy.directory).unwrap();
    assert_eq!(sweep(legacy.directory.parent().unwrap()).unwrap(), 0);
    assert!(last_use_path(&legacy.directory).exists());
    assert!(
        layout::history(&legacy.directory)
            .join("gen-000001")
            .exists()
    );
}

#[test]
fn legacy_history_expires_in_place_without_moving_identity() {
    let (fixture, location) = fixture();
    let legacy = legacy_location(&fixture, &location);
    let mut store = Store::open(legacy.clone()).unwrap();
    let first = store
        .transaction(true, |tx| Ok(tx.point("test", "legacy", "test", false)))
        .unwrap();
    drop(store);
    age(&legacy.directory);
    assert_eq!(sweep(legacy.directory.parent().unwrap()).unwrap(), 1);
    assert!(!legacy.directory.join("gen-000001").exists());
    assert!(legacy.directory.join("workspace.json").exists());
    assert_eq!(layout::version(&legacy.directory), Some(2));
    assert_eq!(
        fs::read(legacy.directory.join("CURRENT"))
            .unwrap()
            .as_slice(),
        b"migrated-to-history"
    );
    let reopened = Store::open(legacy.clone()).unwrap();
    assert!(reopened.index.next > first);
}

#[test]
fn legacy_readers_block_new_writers_until_migration_can_finish() {
    for owner in [false, true] {
        let (_fixture, legacy, id, bytes) = legacy_with_object();
        let journal = legacy.directory.join("gen-000001/000001.log");
        let before = fs::read(&journal).unwrap();
        let reader =
            (!owner).then(|| Shared::acquire(&legacy.directory.join("gen-000001.lock")).unwrap());
        let owner =
            owner.then(|| Exclusive::acquire(&legacy.directory.join("owners/older.lock")).unwrap());
        let error = Store::open(legacy.clone()).err().unwrap();
        assert!(
            error.contains("migration is waiting for older readers"),
            "{error}"
        );
        assert_eq!(layout::version(&legacy.directory), Some(1));
        assert_eq!(fs::read(&journal).unwrap(), before);
        assert!(!legacy.directory.join(".migrating").exists());
        drop(reader);
        drop(owner);
        let reopened = Store::open(legacy.clone()).unwrap();
        assert_eq!(layout::version(&legacy.directory), Some(2));
        assert_eq!(&*reopened.bytes(id).unwrap(), bytes.as_slice());
    }
}

#[test]
fn legacy_migration_reopens_exact_object_bytes() {
    let (_fixture, legacy, id, bytes) = legacy_with_object();
    let reopened = Store::open(legacy.clone()).unwrap();
    assert_eq!(&*reopened.bytes(id).unwrap(), bytes.as_slice());
    assert_eq!(layout::version(&legacy.directory), Some(2));
    assert!(layout::generation_directory(&legacy.directory, 1).exists());
    assert_eq!(
        fs::read(legacy.directory.join("CURRENT"))
            .unwrap()
            .as_slice(),
        b"migrated-to-history"
    );
}

#[test]
fn interrupted_legacy_migration_resumes_before_opening_journal() {
    for after_sentinel in [false, true] {
        let (_fixture, legacy, id, bytes) = legacy_with_object();
        fs::write(legacy.directory.join(".migrating"), b"1 0").unwrap();
        if after_sentinel {
            fs::write(legacy.directory.join("CURRENT"), b"migrated-to-history").unwrap();
            fs::create_dir(legacy.directory.join("history")).unwrap();
            fs::rename(
                legacy.directory.join("gen-000001"),
                legacy.directory.join("history/gen-000001"),
            )
            .unwrap();
        }
        let reopened = Store::open(legacy.clone()).unwrap();
        assert_eq!(&*reopened.bytes(id).unwrap(), bytes.as_slice());
        assert!(!legacy.directory.join(".migrating").exists());
        assert_eq!(layout::version(&legacy.directory), Some(2));
    }
}

#[test]
fn damaged_current_and_unknown_directories_are_preserved() {
    let (_fixture, location) = fixture();
    let store = Store::open(location.clone()).unwrap();
    drop(store);
    age(&location.directory);
    fs::write(layout::current(&location.directory), b"nonsense").unwrap();
    let unknown = location.directory.parent().unwrap().join("unrelated");
    fs::create_dir(&unknown).unwrap();
    fs::write(
        unknown.join("workspace.json"),
        b"{\"version\":2,\"root\":\"C:/outside\"}",
    )
    .unwrap();
    fs::create_dir(unknown.join("history")).unwrap();
    assert_eq!(sweep(location.directory.parent().unwrap()).unwrap(), 0);
    assert!(layout::history(&location.directory).exists());
    assert!(unknown.join("history").exists());
}

#[test]
fn stale_owner_file_does_not_prevent_expiry() {
    let (_fixture, location) = fixture();
    let store = Store::open(location.clone()).unwrap();
    drop(store);
    let owners = layout::owners_directory(&location.directory);
    fs::create_dir_all(&owners).unwrap();
    fs::write(owners.join("stale.lock"), b"").unwrap();
    age(&location.directory);
    assert_eq!(sweep(location.directory.parent().unwrap()).unwrap(), 1);
}

#[test]
fn recent_throttled_action_cannot_expire_early() {
    let (_fixture, location) = fixture();
    let store = Store::open(location.clone()).unwrap();
    drop(store);
    let old = crate::sessions::now() - Duration::from_secs(7 * 24 * 60 * 60).as_millis() as u64 - 1;
    files::atomic_write(
        &last_use_path(&location.directory),
        old.to_string().as_bytes(),
    )
    .unwrap();
    assert_eq!(sweep(location.directory.parent().unwrap()).unwrap(), 0);
    assert!(layout::history(&location.directory).exists());
}
