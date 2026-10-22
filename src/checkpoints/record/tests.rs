use super::*;
use crate::checkpoints::frame::{Encoding, HEADER, ObjectMeta, checksum, header};
use crate::checkpoints::hash;

#[test]
fn records_and_objects_round_trip_with_checksums() {
    let records = [
        Record::Version(Version {
            seq: 9,
            path: "dir/ü.txt".into(),
            state: State::File {
                object: 4,
                length: 3,
                digest: hash::digest(b"abc"),
                mode: Mode(0o755),
            },
            stat: Some(Stat {
                length: 3,
                modified: 7,
                ..Stat::default()
            }),
            time: 11,
            origin: "session".into(),
            operation: 8,
            discovery: Discovery::Created,
        }),
        Record::Prepare(Prepare {
            seq: 8,
            owner: "o".into(),
            origin: "s".into(),
            kind: "edit".into(),
            path: "a".into(),
            before: State::Absent,
            after: State::Link { target: "b".into() },
            time: 1,
        }),
        Record::Event(Event {
            seq: 3,
            origin: "user".into(),
            time: 2,
            text: "restored".into(),
            paths: vec!["a".into(), "b".into()],
        }),
        Record::Coverage {
            seq: 4,
            complete: true,
            reason: "baseline".into(),
        },
    ];
    for record in records {
        let frame = record.frame();
        let header = header(frame[..HEADER].try_into().unwrap()).unwrap();
        let payload = &frame[HEADER..];
        assert_eq!(header.checksum, checksum(header.kind, payload));
        let kind = Kind::from(header.kind).unwrap();
        assert_eq!(Record::decode(kind, payload).unwrap(), record);
    }
    let meta = ObjectMeta {
        id: 5,
        digest: hash::digest(b"xyz"),
        length: 3,
        encoding: Encoding::Delta {
            base: 2,
            prefix: 1,
            suffix: 1,
        },
    };
    let frame = meta.encode(b"y");
    assert_eq!(ObjectMeta::decode(&frame[HEADER..]).unwrap(), meta);
    assert_eq!(&frame[HEADER + ObjectMeta::SIZE..], b"y");
    let mut damaged = frame.clone();
    *damaged.last_mut().unwrap() ^= 1;
    let header = header(damaged[..HEADER].try_into().unwrap()).unwrap();
    assert_ne!(header.checksum, checksum(header.kind, &damaged[HEADER..]));
}
