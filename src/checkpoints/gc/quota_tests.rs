use super::*;
use crate::checkpoints::quota;

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
