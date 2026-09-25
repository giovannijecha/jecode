use super::*;

#[test]
fn older_attempts_load_with_absent_diagnostic_counters() {
    let old = crate::json::parse(
        r#"{"delivery":"possibly_submitted","stage":"response_read","operation":"TLS record body read","category":"ConnectionReset","os_code":10054,"accepted_wire_bytes":159318,"diagnostic":"connection reset","retrying":false}"#,
        Default::default(),
    ).unwrap();
    let attempt = super::codec::read_attempt(&old).unwrap();
    assert_eq!(attempt.accepted_wire_bytes, 159318);
    assert_eq!(attempt.request_sequence, 0);
    assert_eq!(attempt.received_wire_bytes, 0);
    assert_eq!(attempt.response_status, None);
}

#[test]
fn diagnostic_export_is_directory_scoped_and_limited_to_recent_attempts() {
    let fixture = crate::state::tests::Fixture::new();
    let Some(store) = fixture.store() else { return };
    let mut history = create_in(&store, Model::Luna, Some(&fixture.0), None).unwrap();
    let id = history.record.as_ref().unwrap().id().to_owned();
    history.begin("synthetic private prompt".into()).unwrap();
    history.turns[0].steps.push(Step {
        text: "synthetic private response".into(),
        attempts: (1..=40)
            .map(
                |sequence| crate::providers::openai_account::client::Attempt {
                    request_sequence: sequence,
                    ..Default::default()
                },
            )
            .collect(),
        ..Default::default()
    });
    history.checkpoint().unwrap();
    drop(history);
    let directory = crate::session::scope::Directory::open(&fixture.0).unwrap();
    let attempts = recent_network_attempts_in_store(&store, &id, &directory).unwrap();
    assert_eq!(attempts.len(), 32);
    assert_eq!(attempts.first().unwrap().request_sequence, 9);
    assert_eq!(attempts.last().unwrap().request_sequence, 40);
    let elsewhere = crate::state::tests::Fixture::new();
    let other = crate::session::scope::Directory::open(&elsewhere.0).unwrap();
    assert!(recent_network_attempts_in_store(&store, &id, &other).is_err());
}

#[test]
fn interrupted_stream_and_prior_receipt_survive_resume_without_replay() {
    use crate::providers::openai_account::client::{Attempt, Delivery, RequestStage};
    let fixture = crate::state::tests::Fixture::new();
    let Some(store) = fixture.store() else { return };
    let mut history = create(&store, Model::Luna, None).unwrap();
    let id = history.record.as_ref().unwrap().id.clone();
    history.begin("finish the workspace task".into()).unwrap();
    history.turns[0].steps.push(Step {
        response: Some(session::tool_tests::calls_response(vec![
            session::tool_tests::call("read-1", "read_file", r#"{"path":"notes.txt"}"#),
        ])),
        accepted: true,
        results: vec![Receipt {
            call_id: "read-1".into(),
            output: r#"{"text":"prior result"}"#.into(),
            summary: "Read notes.txt".into(),
        }],
        ..Default::default()
    });
    history.turns[0].steps.push(Step {
        text: "unfinished answer".into(),
        attempts: vec![Attempt {
            request_sequence: 2,
            connection_attempt: 1,
            delivery: Delivery::Streaming,
            stage: Some(RequestStage::ResponseRead),
            stage_elapsed_ms: 831,
            operation: Some("TLS record body read".into()),
            category: Some("ConnectionReset".into()),
            os_code: Some(10054),
            accepted_wire_bytes: 91,
            received_wire_bytes: 58,
            response_plaintext_bytes: 36,
            response_status: Some(200),
            stream_events: 1,
            diagnostic: Some("synthetic connection reset / response read".into()),
            retrying: false,
        }],
        ..Default::default()
    });
    history.turns[0].end = Some(End::Failed(Failure::Worker));
    history.turns[0].outcome = "Response interrupted; remote completion uncertain".into();
    history.turns[0].metrics.requests = 1;
    history.turns[0].metrics.connection_attempts = 1;
    history.turns[0].metrics.submissions = 1;
    history.checkpoint().unwrap();
    drop(history);

    let saved = load(&store, &id, true).unwrap();
    let step = &saved.history.turns[0].steps[1];
    assert_eq!(step.text, "unfinished answer");
    assert_eq!(step.attempts[0].delivery, Delivery::Streaming);
    assert_eq!(step.attempts[0].os_code, Some(10054));
    assert_eq!(
        (
            step.attempts[0].request_sequence,
            step.attempts[0].connection_attempt
        ),
        (2, 1)
    );
    assert_eq!(
        (
            step.attempts[0].stage_elapsed_ms,
            step.attempts[0].received_wire_bytes,
            step.attempts[0].response_plaintext_bytes,
            step.attempts[0].response_status,
            step.attempts[0].stream_events
        ),
        (831, 58, 36, Some(200), 1)
    );
    let encoded_attempt =
        crate::json::encode(&super::codec::attempt(&step.attempts[0]), 1024).unwrap();
    assert!(!encoded_attempt.contains("prior result"));
    assert!(!encoded_attempt.contains("unfinished answer"));
    assert!(step.response.is_none());
    assert_eq!(
        (
            saved.history.turns[0].metrics.requests,
            saved.history.turns[0].metrics.connection_attempts,
            saved.history.turns[0].metrics.submissions
        ),
        (1, 1, 1)
    );
    let observed = Arc::new(Mutex::new(Vec::new()));
    let mut run =
        Session::with_history(Model::Luna, Backend(observed.clone()), None, saved.history).unwrap();
    assert!(matches!(
        session::tests::next(&mut run),
        Event::Restored { .. }
    ));
    assert!(matches!(session::tests::next(&mut run), Event::Ready));
    assert!(observed.lock().unwrap().is_empty());
    assert!(run.submit("continue from the recorded state"));
    loop {
        match session::tests::next(&mut run) {
            Event::Finished(End::Complete, metrics) => {
                assert_eq!(metrics.requests, 1);
                break;
            }
            Event::Text(_) => {}
            _ => panic!("historical tool was replayed"),
        }
    }
    let requests = observed.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert!(requests[0].contains("prior result"));
    assert!(requests[0].contains("unfinished answer"));
    assert!(requests[0].contains("streaming_unvalidated"));
    assert!(requests[0].contains("continue from the recorded state"));
}
