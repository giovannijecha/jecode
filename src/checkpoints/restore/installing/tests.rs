use super::*;
use crate::checkpoints::identity;
use crate::test_support::Directory;

fn archive_error_is_not_a_live_conflict(bytes: Vec<u8>) {
    let home = Directory::new();
    let root = Directory::new();
    let path = root.path().join("file.bin");
    std::fs::write(&path, &bytes).unwrap();
    let location = identity::locate(home.path(), root.path(), true)
        .unwrap()
        .unwrap();
    let mut store = Store::open(location.clone()).unwrap();
    let (object, digest) = store
        .transaction(true, |transaction| {
            transaction.object_reader(std::io::Cursor::new(&bytes), bytes.len() as u64, None)
        })
        .unwrap();
    drop(store);
    let store = Store::open(location.clone()).unwrap();
    let mode = files::Mode::of(&std::fs::metadata(&path).unwrap());
    let planned = Planned {
        path: "file.bin".into(),
        action: super::super::Action::Delete,
        target: State::Absent,
        current: State::File {
            object,
            length: bytes.len() as u64,
            digest,
            mode,
        },
        current_seq: object,
    };

    let mut changed = bytes.clone();
    changed[0] ^= 1;
    std::fs::write(&path, &changed).unwrap();
    assert!(matches!(
        install(&store, root.path(), &planned, u64::MAX),
        Err(Failure::Changed(_))
    ));
    assert_eq!(std::fs::read(&path).unwrap(), changed);
    drop(store);

    // Reopening clears verified object caches before the archived data fails.
    std::fs::write(&path, &bytes).unwrap();
    let store = Store::open(location).unwrap();
    std::fs::OpenOptions::new()
        .write(true)
        .open(store.journal.segment_path(1))
        .unwrap()
        .set_len(0)
        .unwrap();
    assert!(
        store
            .equal_reader(object, std::io::Cursor::new(&bytes))
            .is_err()
    );
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    assert!(matches!(
        install(&store, root.path(), &planned, u64::MAX),
        Err(Failure::Io(_))
    ));
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
}

#[test]
fn unreadable_inline_archive_is_an_io_failure() {
    archive_error_is_not_a_live_conflict(b"unchanged live content".to_vec());
}

#[test]
fn unreadable_chunked_archive_is_an_io_failure() {
    let mut bytes = vec![b'x'; 3 * crate::checkpoints::store::CHUNK_BYTES + 17];
    for (number, chunk) in bytes
        .chunks_mut(crate::checkpoints::store::CHUNK_BYTES)
        .enumerate()
    {
        chunk[0] = number as u8;
    }
    archive_error_is_not_a_live_conflict(bytes);
}
