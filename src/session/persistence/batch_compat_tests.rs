use super::*;
use crate::session::{
    End,
    history::{Receipt, Step},
    tool_tests,
};

fn paired_step(count: usize) -> Step {
    let calls = (0..count)
        .map(|n| {
            tool_tests::call(
                &format!("read-{n}"),
                "read_file",
                r#"{"path":"source.txt"}"#,
            )
        })
        .collect::<Vec<_>>();
    let results = calls
        .iter()
        .enumerate()
        .map(|(n, call)| Receipt {
            call_id: call.id.clone(),
            output: format!("EXACT-{n}"),
            summary: "read_file / source.txt".into(),
            image: None,
        })
        .collect();
    Step {
        response: Some(tool_tests::calls_response(calls)),
        results,
        accepted: true,
        ..Default::default()
    }
}

#[test]
fn v1_snapshot_128_and_129_receipts_remain_readable_without_rewrite() {
    for count in [128, 129] {
        let fixture = crate::state::tests::Fixture::new();
        let store = fixture.store().unwrap();
        let mut history = create(&store, Model::Luna, None).unwrap();
        let id = history.record.as_ref().unwrap().id().to_owned();
        history.begin("Legacy batch".into()).unwrap();
        history.turns[0].steps.push(paired_step(count));
        history.turns[0].end = Some(End::Complete);
        history.checkpoint().unwrap();
        drop(history);
        let sessions = store.directory("sessions").unwrap();
        let name = format!("{id}.json");
        let original = sessions.read(&name, LIMIT).unwrap().unwrap();
        let saved = load(&store, &id, true).unwrap();
        assert!(saved.history.record.as_ref().unwrap().legacy());
        let step = &saved.history.turns[0].steps[0];
        assert_eq!(step.results.len(), count);
        for n in [0, count / 2, count - 1] {
            assert_eq!(
                step.response.as_ref().unwrap().tool_calls[n].id,
                format!("read-{n}")
            );
            assert_eq!(step.results[n].call_id, format!("read-{n}"));
            assert_eq!(step.results[n].output, format!("EXACT-{n}"));
        }
        drop(saved);
        assert_eq!(sessions.read(&name, LIMIT).unwrap().unwrap(), original);
    }
}

#[test]
fn v2_rejects_mismatched_pair_before_committing_a_head() {
    let fixture = crate::state::tests::Fixture::new();
    let store = fixture.store().unwrap();
    let mut history = v2::create(&store, Model::Luna, None, None).unwrap();
    let id = history.record.as_ref().unwrap().id().to_owned();
    history.begin("Paired batch".into()).unwrap();
    history.checkpoint().unwrap();
    let directory = store.directory("sessions-v2").unwrap();
    let name = format!("{id}.head");
    let before = directory.read(&name, LIMIT).unwrap().unwrap();
    let mut malformed = paired_step(129);
    malformed.results[64].call_id = "different-call".into();
    history.turns[0].steps.push(malformed);
    assert!(history.checkpoint().is_err());
    assert_eq!(directory.read(&name, LIMIT).unwrap().unwrap(), before);
    drop(history);
    let saved = load(&store, &id, true).unwrap();
    assert!(saved.history.turns[0].steps.is_empty());
}

#[test]
fn v2_rejects_a_receipt_exceeding_its_decoder_bound_before_commit() {
    let fixture = crate::state::tests::Fixture::new();
    let store = fixture.store().unwrap();
    let mut history = v2::create(&store, Model::Luna, None, None).unwrap();
    let id = history.record.as_ref().unwrap().id().to_owned();
    history.begin("Bounded batch".into()).unwrap();
    history.checkpoint().unwrap();
    let directory = store.directory("sessions-v2").unwrap();
    let name = format!("{id}.head");
    let before = directory.read(&name, LIMIT).unwrap().unwrap();
    let mut too_large = paired_step(129);
    too_large.results[64].output = "x".repeat(crate::session::history::MAX_TEXT + 1);
    history.turns[0].steps.push(too_large);
    assert!(history.checkpoint().is_err());
    assert_eq!(directory.read(&name, LIMIT).unwrap().unwrap(), before);
    drop(history);
    assert!(
        load(&store, &id, true).unwrap().history.turns[0]
            .steps
            .is_empty()
    );
}
