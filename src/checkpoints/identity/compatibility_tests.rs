use super::*;
use crate::checkpoints::{layout, store::Store};
use crate::test_support::Directory;

fn fixture() -> (Directory, PathBuf, PathBuf, PathBuf) {
    let fixture = Directory::new();
    let home = fixture.path().join("home");
    let root = fixture.path().join("project");
    fs::create_dir(&root).unwrap();
    let root = fs::canonicalize(root).unwrap();
    let hash = identity_hash(&root.to_string_lossy());
    let legacy = home.join("checkpoints").join(format!("{hash:032x}"));
    (fixture, home, root, legacy)
}

#[test]
fn fresh_readable_archive_reserves_legacy_lookup_and_rejects_flat_journals() {
    let (_fixture, home, root, legacy) = fixture();
    let location = locate(&home, &root, true).unwrap().unwrap();
    let reserved = compatibility::reservation(&legacy).unwrap().unwrap();
    assert_eq!(reserved.root, root.to_string_lossy());
    assert_eq!(
        reserved.archive,
        location.directory.file_name().unwrap().to_string_lossy()
    );
    // This is the filesystem operation used by the old hash-only locator.
    assert!(files::private_directory(&legacy).is_err());
    assert_eq!(
        fs::read(location.directory.join("CURRENT")).unwrap(),
        layout::LEGACY_SENTINEL
    );
    assert!(
        fs::read_to_string(location.directory.join("CURRENT"))
            .unwrap()
            .parse::<u64>()
            .is_err()
    );
    let mut store = Store::open(location.clone()).unwrap();
    let point = store
        .transaction(true, |tx| Ok(tx.point("test", "new", "test", true)))
        .unwrap();
    drop(store);
    let again = locate(&home, &root, true).unwrap().unwrap();
    assert_eq!(again.directory, location.directory);
    assert!(
        Store::open(again)
            .unwrap()
            .index
            .points
            .contains_key(&point)
    );
}

#[test]
fn interrupted_reservation_resumes_before_or_after_directory_creation() {
    for created in [false, true] {
        let (_fixture, home, root, legacy) = fixture();
        let hash = identity_hash(&root.to_string_lossy());
        let target = home
            .join("checkpoints")
            .join(format!("project-{hash:032x}"));
        compatibility::reserve(&legacy, &root.to_string_lossy(), &target).unwrap();
        if created {
            fs::create_dir(&target).unwrap();
        }
        assert!(locate(&home, &root, false).unwrap().is_none());
        let location = locate(&home, &root, true).unwrap().unwrap();
        assert_eq!(location.directory, target);
        assert_eq!(layout::version(&target), Some(2));
        Store::open(location).unwrap();
    }
}

#[test]
fn guardless_v2_is_repaired_without_changing_identity_or_history() {
    let (_fixture, home, root, legacy) = fixture();
    let location = locate(&home, &root, true).unwrap().unwrap();
    let mut store = Store::open(location.clone()).unwrap();
    let point = store
        .transaction(true, |tx| Ok(tx.point("test", "existing", "test", true)))
        .unwrap();
    drop(store);
    let identity = fs::read(location.directory.join("workspace.json")).unwrap();
    fs::remove_file(&legacy).unwrap();
    fs::remove_file(location.directory.join("CURRENT")).unwrap();
    let again = locate(&home, &root, true).unwrap().unwrap();
    assert_eq!(again.directory, location.directory);
    assert_eq!(
        fs::read(again.directory.join("workspace.json")).unwrap(),
        identity
    );
    assert!(
        Store::open(again)
            .unwrap()
            .index
            .points
            .contains_key(&point)
    );
    assert!(legacy.is_file());
}

#[test]
fn parallel_flat_history_is_preserved_and_never_hidden_by_a_sentinel() {
    for entry in ["CURRENT", "gen-000001", "owners"] {
        let (_fixture, home, root, _legacy) = fixture();
        let location = locate(&home, &root, true).unwrap().unwrap();
        fs::remove_file(location.directory.join("CURRENT")).unwrap();
        let old = location.directory.join(entry);
        if entry == "CURRENT" {
            fs::write(&old, b"1").unwrap();
        } else {
            fs::create_dir(&old).unwrap();
            fs::write(old.join("evidence"), b"preserve legacy data").unwrap();
        }
        assert!(locate(&home, &root, true).is_err());
        assert!(Store::open(location.clone()).is_err());
        if entry == "CURRENT" {
            assert_eq!(fs::read(&old).unwrap(), b"1");
        } else {
            assert_eq!(
                fs::read(old.join("evidence")).unwrap(),
                b"preserve legacy data"
            );
        }
    }
}

#[test]
fn invalid_reservations_are_not_overwritten_and_rejected_old_opens_can_reopen() {
    let (_fixture, home, root, legacy) = fixture();
    files::private_directory(legacy.parent().unwrap()).unwrap();
    fs::write(&legacy, b"unrecognized data").unwrap();
    assert!(locate(&home, &root, true).is_err());
    assert_eq!(fs::read(&legacy).unwrap(), b"unrecognized data");
    fs::remove_file(&legacy).unwrap();
    let location = locate(&home, &root, true).unwrap().unwrap();
    // An older Unix opener registers its owner before checking root CURRENT.
    fs::create_dir(location.directory.join("owners")).unwrap();
    assert!(locate(&home, &root, true).is_ok());
    Store::open(location).unwrap();
}
