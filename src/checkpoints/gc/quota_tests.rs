use super::*;
use crate::cancel::Cancellation;
use crate::checkpoints::identity;
use crate::checkpoints::policy::Policy;
use crate::checkpoints::quota;
use crate::checkpoints::tests::Fixture;
use std::path::Path;

fn orphan_space(archive: &Path, _: Option<&Cancellation>) -> Result<u64, String> {
    let marker = journal::generation_directory(archive, 2).join("orphan.bin");
    Ok(if marker.exists() { 0 } else { u64::MAX / 2 })
}

fn modest_space(_: &Path, _: Option<&Cancellation>) -> Result<u64, String> {
    Ok(8 << 20)
}

#[test]
fn unfinished_generation_is_removed_before_scratch_admission() {
    let fixture = Fixture::new();
    drop(fixture.open());
    let location = identity::locate(fixture.home.path(), fixture.root.path(), false)
        .unwrap()
        .unwrap();
    let orphan = journal::generation_directory(&location.directory, 2).join("orphan.bin");
    std::fs::create_dir_all(orphan.parent().unwrap()).unwrap();
    std::fs::write(&orphan, b"interrupted collection").unwrap();

    let mut store = Store::open(location.clone()).unwrap();
    store.journal.set_test_probe(orphan_space);
    let collection = collect_opened(&location, store, true).unwrap().unwrap();
    assert_eq!(collection.generation, 2);
    assert!(!orphan.exists());
}

#[test]
fn scratch_admission_uses_configured_free_space_minimum() {
    let fixture = Fixture::new();
    drop(fixture.open());
    let location = identity::locate(fixture.home.path(), fixture.root.path(), false)
        .unwrap()
        .unwrap();
    let mut policy = Policy::read(&location.directory).unwrap().unwrap();
    policy.min_free_bytes = 1 << 20;
    policy.save(&location.directory).unwrap();
    // Adoption writes a policy record and updates that handle's journal.
    // Reopen as collection does when the policy is already durable.
    drop(Store::open(location.clone()).unwrap());

    let mut store = Store::open(location.clone()).unwrap();
    store.journal.set_test_probe(modest_space);
    let collection = collect_opened(&location, store, true).unwrap().unwrap();
    assert_eq!(collection.generation, 2);
}

#[test]
fn unfinished_copy_cannot_double_an_old_generation_past_the_cap() {
    let directory = crate::test_support::Directory::new();
    let archive = directory.path();
    let old = journal::generation_directory(archive, 1);
    std::fs::create_dir(&old).unwrap();
    let old_segment = old.join("000001.log");
    std::fs::write(&old_segment, vec![b'x'; 300_000]).unwrap();
    let before = quota::used(archive).unwrap();
    let mut journal = Journal::open_generation(archive, 2).unwrap();
    journal.quota_limit = before + quota::RESERVE + 128;
    let mut output = Output {
        journal,
        lock_path: archive.join("store.lock"),
        under_store_lock: false,
        buffer: Vec::new(),
        bytes: 0,
    };
    output.raw(&vec![b'y'; 1024]).unwrap();
    let error = output.finish().unwrap_err();
    assert!(quota::is_full(&error), "{error}");
    assert_eq!(quota::used(archive).unwrap(), before);
    assert_eq!(std::fs::metadata(old_segment).unwrap().len(), 300_000);
}
