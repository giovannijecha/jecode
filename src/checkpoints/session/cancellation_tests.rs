use super::*;
use crate::cancel::Cancellation;
use crate::checkpoints::recovery;
use crate::checkpoints::restore::{Expected, Request};
use crate::test_support::Directory;
use std::fs;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

#[test]
fn cancelled_prepared_write_releases_its_lease_and_keeps_recovery() {
    let home = Directory::new();
    let root = Directory::new();
    let path = root.path().join("notes.txt");
    fs::write(&path, "before").unwrap();
    let session = Session::offline(home.path(), root.path());
    session.begin("cancelled-write", None);
    let workspace = Arc::clone(session.workspace().unwrap());
    let cancellation = Cancellation::default();
    let (prepared_tx, prepared_rx) = mpsc::channel();
    let (drop_tx, drop_rx) = mpsc::channel();
    let (dropped_tx, dropped_rx) = mpsc::channel();
    let worker = thread::spawn({
        let session = Arc::clone(&session);
        let cancellation = cancellation.clone();
        let path = path.clone();
        move || {
            let mut guard = session.file_cancellable("write", &path, &cancellation);
            guard.prepare(b"replacement");
            prepared_tx.send(()).unwrap();
            drop_rx.recv().unwrap();
            cancellation.cancel();
            drop(guard);
            dropped_tx.send(()).unwrap();
        }
    });
    prepared_rx.recv_timeout(Duration::from_secs(10)).unwrap();
    let group = {
        let store = workspace.store();
        assert_eq!(store.index.pending.len(), 1);
        let group = recovery::group_for(&store.index, "cancelled-write").unwrap();
        assert!(recovery::target(&store.index, group, "notes.txt").is_some());
        group
    };
    let locked_store = workspace.store();
    drop_tx.send(()).unwrap();
    let dropped = dropped_rx.recv_timeout(Duration::from_secs(2));
    drop(locked_store);
    worker.join().unwrap();
    assert!(dropped.is_ok(), "cancellation waited for the archive lock");
    assert_eq!(fs::read_to_string(&path).unwrap(), "before");

    workspace.flush("external").unwrap();
    assert!(workspace.store().index.pending.is_empty());
    fs::write(&path, "later").unwrap();
    workspace.verify().unwrap();
    let selected = vec!["notes.txt".to_string()];
    workspace
        .restore(&Request {
            target: group,
            paths: Some(&selected),
            exact: true,
            expected: Expected::Latest,
            preview: false,
            origin: "cancelled-write",
            excluded: &workspace.excluded,
        })
        .unwrap();
    assert_eq!(fs::read_to_string(&path).unwrap(), "before");
}
