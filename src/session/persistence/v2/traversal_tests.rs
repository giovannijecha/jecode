use super::*;
use crate::session::{
    End,
    history::{Receipt, Step},
};

#[test]
fn large_old_turn_traverses_exact_ordered_event_slices_with_a_resumable_cursor() {
    let fixture = crate::state::tests::Fixture::new();
    let Some(store) = fixture.store() else { return };
    let mut history = create(&store, Model::Luna, None, None).unwrap();
    let id = history.record.as_ref().unwrap().id().to_owned();
    history.begin("large canonical turn".into()).unwrap();
    for index in 0..86 {
        let call_id = format!("read-{index:02}");
        let call = crate::session::tool_tests::call(
            &call_id,
            "read_file",
            &format!(r#"{{"path":"part-{index:02}.txt"}}"#),
        );
        history.turns[0].steps.push(Step {
            response: Some(crate::session::tool_tests::calls_response(vec![call])),
            accepted: true,
            results: vec![Receipt {
                call_id,
                output: format!("MARKER-{index:02}-{}", "x".repeat(1_000_000)),
                summary: format!("Read part {index:02}"),
            }],
            ..Default::default()
        });
        history.checkpoint().unwrap();
        history.projection.step = 1;
        history.projection.summary = format!("Through part {index:02}");
        history.checkpoint().unwrap();
        history.release_projected();
    }
    history.turns[0].end = Some(End::Complete);
    history.turns[0].outcome = "Complete".into();
    history.checkpoint().unwrap();
    drop(history);
    let mut saved = super::super::load(&store, &id, true).unwrap();
    assert!(saved.transcript_page(0, 1).is_err());
    let v2 = store.directory("sessions-v2").unwrap();
    let info = head::read(&v2, &id).unwrap();
    let mut expected = Vec::new();
    log::visit(&v2, &id, info.committed, info.rolling, 0, 1, |_, value| {
        let encoded = json::encode(&value, log::EVENT_LIMIT).unwrap();
        expected.push((
            encoded.len(),
            test_hash(log::HASH_START, encoded.as_bytes()),
        ));
        Ok(())
    })
    .unwrap();
    let mut cursor = None;
    let mut first_cursor = None;
    let mut event = 0usize;
    let mut offset = 0usize;
    let mut digest = log::HASH_START;
    let mut covered = 0u64;
    let mut pages = 0;
    let mut split_event = false;
    loop {
        let page = saved
            .canonical_turn_slices(0, cursor, 8 * 1024 * 1024)
            .unwrap();
        pages += 1;
        assert!(page.slices.len() <= 64);
        assert!(
            page.slices
                .iter()
                .map(|slice| slice.bytes.len())
                .sum::<usize>()
                <= 8 * 1024 * 1024
        );
        for slice in page.slices {
            assert_eq!(slice.event, event);
            assert_eq!(slice.offset, offset);
            assert_eq!(slice.total, expected[event].0);
            digest = test_hash(digest, &slice.bytes);
            offset += slice.bytes.len();
            covered += slice.bytes.len() as u64;
            if offset == slice.total {
                assert_eq!(digest, expected[event].1);
                event += 1;
                offset = 0;
                digest = log::HASH_START;
            } else {
                split_event = true;
            }
        }
        assert_eq!(
            page.total_bytes,
            expected.iter().map(|entry| entry.0 as u64).sum()
        );
        if first_cursor.is_none() {
            first_cursor = page.next;
        }
        cursor = page.next;
        if cursor.is_none() {
            break;
        }
        if pages == 1 {
            drop(saved);
            saved = super::super::load(&store, &id, true).unwrap();
        }
    }
    eprintln!(
        "large turn traversal: events={} bytes={covered} pages={pages}",
        expected.len()
    );
    assert!(covered > 80 * 1024 * 1024);
    assert!(pages > 1 && split_event);
    assert_eq!(event, expected.len());
    saved.history.begin("later turn".into()).unwrap();
    saved.history.checkpoint().unwrap();
    assert!(
        saved
            .canonical_turn_slices(0, first_cursor, 8 * 1024 * 1024)
            .is_err()
    );
}

fn test_hash(mut value: u64, bytes: &[u8]) -> u64 {
    for byte in bytes {
        value ^= u64::from(*byte);
        value = value.wrapping_mul(0x100000001b3);
    }
    value
}
