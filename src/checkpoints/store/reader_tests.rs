use super::*;

#[test]
fn payload_reader_keeps_its_lease_after_loaded_metadata_is_dropped() {
    let fixture = Fixture::new();
    fs::write(fixture.workspace().join("data"), b"before").unwrap();
    let store = fixture.store();
    let cancel = Cancellation::default();
    let captured = store
        .capture("write", "test", &["data".into()], &cancel)
        .unwrap();
    let reference = captured.reference.unwrap();
    let loaded = store.load(&reference, &cancel).unwrap();
    let mut reader = loaded.reader(&loaded.snapshot.entries["data"]).unwrap();
    drop(loaded);
    assert_eq!(store.cleanup(captured.created + TTL_MILLIS).unwrap(), 0);
    let mut bytes = Vec::new();
    reader.read_to_end(&mut bytes).unwrap();
    assert_eq!(bytes, b"before");
    drop(reader);
    assert_eq!(store.cleanup(captured.created + TTL_MILLIS).unwrap(), 1);
}

#[cfg(unix)]
#[test]
fn inherited_descriptor_does_not_extend_the_reader_lease() {
    let fixture = Fixture::new();
    fs::write(fixture.workspace().join("data"), b"before").unwrap();
    let store = fixture.store();
    let cancel = Cancellation::default();
    let captured = store
        .capture("write", "test", &["data".into()], &cancel)
        .unwrap();
    let reference = captured.reference.unwrap();
    let loaded = store.load(&reference, &cancel).unwrap();
    // A fork inherits another descriptor for the same open file description.
    let inherited = loaded.lease.file.try_clone().unwrap();
    drop(loaded);
    assert_eq!(store.cleanup(captured.created + TTL_MILLIS).unwrap(), 1);
    drop(inherited);
}
