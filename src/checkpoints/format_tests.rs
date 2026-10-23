use super::*;
use crate::test_support::Directory;

#[test]
fn previous_pack_formats_keep_their_saved_payloads() {
    let directory = Directory::new();
    let payload = b"before";
    let state = State::File {
        offset: HEADER,
        length: payload.len() as u64,
        digest: hash::digest(payload),
        mode: Mode(0o644),
    };
    let snapshot = Snapshot {
        reference: "rc-0000000000000001-00000001-0000000000000001".into(),
        created: 1,
        tool: "write".into(),
        origin: "test".into(),
        entries: [("data".into(), state.clone())].into_iter().collect(),
        roots: Vec::new(),
        excluded: Vec::new(),
        ignored: Vec::new(),
        ignore_rules: super::super::paths::IgnoreRules::default(),
        partial: Vec::new(),
    };
    for (magic, end, added_fields) in [
        (b"JECRCP02", b"RCPEND02", 12),
        (b"JECRCP03", b"RCPEND03", 8),
    ] {
        let mut snapshot = snapshot.clone();
        if magic == b"JECRCP03" {
            snapshot.ignored.push("cache".into());
        }
        let mut metadata = encode(&snapshot).unwrap();
        // Empty appended scope fields are four-byte counts. Older packs
        // finish with only their original fields and the empty issue count.
        metadata.truncate(metadata.len() - added_fields);
        let mut file = crate::disk::private_options()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(directory.path().join("previous.rcp"))
            .unwrap();
        file.write_all(magic).unwrap();
        file.write_all(&snapshot.created.to_le_bytes()).unwrap();
        file.write_all(&[0; 16]).unwrap();
        file.write_all(payload).unwrap();
        file.write_all(&metadata).unwrap();
        file.write_all(&(metadata.len() as u64).to_le_bytes())
            .unwrap();
        file.write_all(&hash::checksum(&[magic, &metadata]).to_le_bytes())
            .unwrap();
        file.write_all(end).unwrap();
        let decoded = read(&mut file).unwrap();
        assert_eq!(decoded.reference, snapshot.reference);
        assert_eq!(decoded.entries["data"], state);
        assert_eq!(decoded.ignored, snapshot.ignored);
        assert!(decoded.ignore_rules.local.is_empty());
        file.seek(SeekFrom::Start(HEADER)).unwrap();
        let mut restored = [0; 6];
        file.read_exact(&mut restored).unwrap();
        assert_eq!(&restored, payload);
    }
}
