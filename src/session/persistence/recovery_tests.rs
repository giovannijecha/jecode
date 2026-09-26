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
    assert_eq!(attempt.request_elapsed_ms, 0);
    assert_eq!(attempt.since_progress_ms, None);
    assert_eq!(attempt.termination, None);
}

#[test]
fn timing_diagnostics_round_trip_without_request_content() {
    use crate::providers::openai_account::client::{Attempt, Delivery, RequestStage};
    let attempt = Attempt {
        delivery: Delivery::Streaming,
        stage: Some(RequestStage::ResponseRead),
        stage_elapsed_ms: 301_000,
        request_elapsed_ms: 315_000,
        since_progress_ms: Some(300_000),
        termination: Some(crate::providers::openai_account::client::Termination::IdleTimeout),
        accepted_wire_bytes: 512,
        response_status: Some(200),
        stream_events: 17,
        ..Default::default()
    };
    let encoded = super::codec::attempt(&attempt);
    assert_eq!(super::codec::read_attempt(&encoded).unwrap(), attempt);
    let body = crate::json::encode(&encoded, 2048).unwrap();
    assert!(body.contains("stream_idle_timeout"));
    assert!(!body.contains("prompt"));
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
    assert_eq!(attempts.first().unwrap().attempt.request_sequence, 9);
    assert_eq!(attempts.last().unwrap().attempt.request_sequence, 40);
    let elsewhere = crate::state::tests::Fixture::new();
    let other = crate::session::scope::Directory::open(&elsewhere.0).unwrap();
    assert!(recent_network_attempts_in_store(&store, &id, &other).is_err());
}

#[test]
fn diagnostic_export_does_not_commit_interrupted_outcome() {
    let fixture = crate::state::tests::Fixture::new();
    let Some(store) = fixture.store() else { return };
    let mut history = create_in(&store, Model::Luna, Some(&fixture.0), None).unwrap();
    let id = history.record.as_ref().unwrap().id().to_owned();
    history.begin("active when process exited".into()).unwrap();
    history.turns[0].steps.push(Step {
        response: Some(session::tool_tests::calls_response(vec![
            session::tool_tests::call("read-1", "read_file", r#"{"path":"notes.txt"}"#),
        ])),
        accepted: true,
        results: vec![Receipt {
            call_id: "read-1".into(),
            output: "synthetic receipt".into(),
            summary: "Read notes.txt".into(),
            image: None,
        }],
        ..Default::default()
    });
    history.checkpoint().unwrap();
    drop(history);

    let sessions = store.directory("sessions-v2").unwrap();
    let head_name = format!("{id}.head");
    let log_name = format!("{id}.log");
    let before_head = sessions.read(&head_name, 1024 * 1024).unwrap().unwrap();
    let before_log = std::fs::read(sessions.root().join(&log_name)).unwrap();
    let directory = crate::session::scope::Directory::open(&fixture.0).unwrap();
    for _ in 0..2 {
        assert!(
            recent_network_attempts_in_store(&store, &id, &directory)
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            before_head,
            sessions.read(&head_name, 1024 * 1024).unwrap().unwrap(),
            "diagnostic export rewrote the committed session head"
        );
        assert_eq!(
            before_log,
            std::fs::read(sessions.root().join(&log_name)).unwrap()
        );
    }
    let unleased = load(&store, &id, false).unwrap();
    assert!(unleased.history.turns[0].end.is_none());
    assert_eq!(
        unleased.history.turns[0].steps[0].results[0].output,
        "synthetic receipt"
    );
    drop(unleased);

    let recovered = resume_in_store(&store, &id, &directory).unwrap();
    assert_eq!(
        recovered.history.turns[0].end,
        Some(End::Failed(Failure::Worker))
    );
    assert_eq!(
        recovered.history.turns[0].steps[0].results[0].output,
        "synthetic receipt"
    );
    drop(recovered);
    assert_ne!(
        before_head,
        sessions.read(&head_name, 1024 * 1024).unwrap().unwrap()
    );
    assert_ne!(
        before_log,
        std::fs::read(sessions.root().join(&log_name)).unwrap()
    );
}

#[test]
fn diagnostic_export_includes_latest_compaction_failure() {
    use crate::providers::openai_account::client::{Attempt, Delivery, RequestStage};
    let fixture = crate::state::tests::Fixture::new();
    let Some(store) = fixture.store() else { return };
    let mut history = create_in(&store, Model::Luna, Some(&fixture.0), None).unwrap();
    let id = history.record.as_ref().unwrap().id().to_owned();
    history.begin("task".into()).unwrap();
    history.turns[0].steps.push(Step {
        attempts: vec![Attempt {
            request_sequence: 1,
            delivery: Delivery::Completed,
            ..Default::default()
        }],
        ..Default::default()
    });
    history.turns[0].end = Some(End::Failed(Failure::Worker));
    history.projection.failed = true;
    history.projection.failed_attempts = vec![Attempt {
        request_sequence: 2,
        delivery: Delivery::PossiblySubmitted,
        stage: Some(RequestStage::ResponseRead),
        os_code: Some(10054),
        ..Default::default()
    }];
    history.checkpoint().unwrap();
    drop(history);

    let directory = crate::session::scope::Directory::open(&fixture.0).unwrap();
    let attempts = recent_network_attempts_in_store(&store, &id, &directory).unwrap();
    assert!(
        attempts
            .iter()
            .any(|record| record.attempt.os_code == Some(10054)),
        "export omitted latest failure: {attempts:?}"
    );
}

#[test]
fn diagnostic_export_excludes_active_and_foreign_sessions_without_writing() {
    let fixture = crate::state::tests::Fixture::new();
    let Some(store) = fixture.store() else { return };
    let mut history = create_in(&store, Model::Luna, Some(&fixture.0), None).unwrap();
    let id = history.record.as_ref().unwrap().id().to_owned();
    history.begin("active".into()).unwrap();
    history.checkpoint().unwrap();
    let sessions = store.directory("sessions-v2").unwrap();
    let head = format!("{id}.head");
    let log = format!("{id}.log");
    let before_head = sessions.read(&head, 1024 * 1024).unwrap().unwrap();
    let before_log = std::fs::read(sessions.root().join(&log)).unwrap();
    let directory = crate::session::scope::Directory::open(&fixture.0).unwrap();
    assert!(recent_network_attempts_in_store(&store, &id, &directory).is_err());
    drop(history);

    let elsewhere = crate::state::tests::Fixture::new();
    let foreign = crate::session::scope::Directory::open(&elsewhere.0).unwrap();
    assert!(recent_network_attempts_in_store(&store, &id, &foreign).is_err());
    assert_eq!(
        before_head,
        sessions.read(&head, 1024 * 1024).unwrap().unwrap()
    );
    assert_eq!(
        before_log,
        std::fs::read(sessions.root().join(&log)).unwrap()
    );
}

#[test]
fn diagnostic_export_orders_and_bounds_manual_and_automatic_compaction() {
    use crate::providers::openai_account::client::{Attempt, Delivery};
    for manual in [false, true] {
        let fixture = crate::state::tests::Fixture::new();
        let Some(store) = fixture.store() else {
            continue;
        };
        let mut history = create_in(&store, Model::Luna, Some(&fixture.0), None).unwrap();
        let id = history.record.as_ref().unwrap().id().to_owned();
        history.begin("synthetic private prompt".into()).unwrap();
        history.turns[0].steps.push(Step {
            text: "synthetic private response".into(),
            attempts: (1..=34)
                .map(|request_sequence| Attempt {
                    request_sequence,
                    delivery: Delivery::Completed,
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        });
        if manual {
            history.turns[0].end = Some(End::Complete);
            history.checkpoint().unwrap();
        }
        assert_eq!(
            session::context::failed_attempt(
                &mut history,
                Failure::Worker,
                vec![
                    Attempt {
                        request_sequence: 1,
                        delivery: Delivery::NotSubmitted,
                        ..Default::default()
                    },
                    Attempt {
                        request_sequence: 1,
                        delivery: Delivery::PossiblySubmitted,
                        os_code: Some(10054),
                        ..Default::default()
                    },
                ],
                "synthetic private partial summary".into(),
            ),
            Failure::Worker
        );
        if !manual {
            history.turns[0].end = Some(End::Failed(Failure::Worker));
            history.checkpoint().unwrap();
        }
        drop(history);

        let directory = crate::session::scope::Directory::open(&fixture.0).unwrap();
        let attempts = recent_network_attempts_in_store(&store, &id, &directory).unwrap();
        assert_eq!(attempts.len(), 32);
        assert_eq!(attempts[0].source, AttemptSource::Generation);
        assert_eq!(attempts[0].attempt.request_sequence, 5);
        assert_eq!(attempts[29].attempt.request_sequence, 34);
        assert_eq!(attempts[30].source, AttemptSource::Compaction);
        assert_eq!(attempts[31].source, AttemptSource::Compaction);
        assert_eq!(attempts[30].attempt.request_sequence, 1);
        assert_eq!(attempts[31].attempt.request_sequence, 1);
        assert_eq!(attempts[31].attempt.os_code, Some(10054));
        assert!(attempts.iter().all(|record| record.turn == 1));
    }
}

#[test]
fn diagnostic_export_does_not_attach_stale_compaction_to_later_turn() {
    use crate::providers::openai_account::client::Attempt;
    for scoped in [false, true] {
        let fixture = crate::state::tests::Fixture::new();
        let Some(store) = fixture.store() else {
            continue;
        };
        let mut history = create_in(&store, Model::Luna, Some(&fixture.0), None).unwrap();
        let id = history.record.as_ref().unwrap().id().to_owned();
        history.begin("first".into()).unwrap();
        history.turns[0].end = Some(End::Complete);
        history.checkpoint().unwrap();
        let failure = Attempt {
            os_code: Some(10054),
            ..Default::default()
        };
        if scoped {
            session::context::failed_attempt(
                &mut history,
                Failure::Worker,
                vec![failure],
                String::new(),
            );
        } else {
            history.projection.failed = true;
            history.projection.failed_attempts = vec![failure];
            history.checkpoint().unwrap();
        }
        history.begin("later unrelated turn".into()).unwrap();
        history.turns[1].steps.push(Step {
            attempts: vec![Attempt {
                request_sequence: 1,
                ..Default::default()
            }],
            ..Default::default()
        });
        history.turns[1].end = Some(End::Complete);
        history.checkpoint().unwrap();
        drop(history);

        let directory = crate::session::scope::Directory::open(&fixture.0).unwrap();
        let attempts = recent_network_attempts_in_store(&store, &id, &directory).unwrap();
        assert_eq!(attempts.len(), 1);
        assert_eq!(attempts[0].source, AttemptSource::Generation);
        assert_eq!(attempts[0].turn, 2);
        assert_eq!(attempts[0].attempt.os_code, None);
    }
}

#[test]
fn diagnostic_export_preserves_v1_compatibility_without_failure_scope() {
    use crate::providers::openai_account::client::Attempt;
    let fixture = crate::state::tests::Fixture::new();
    let Some(store) = fixture.store() else { return };
    let mut history = create_legacy_in(&store, Model::Luna, Some(&fixture.0), None).unwrap();
    let id = history.record.as_ref().unwrap().id().to_owned();
    history.begin("v1 prompt".into()).unwrap();
    history.turns[0].steps.push(Step {
        attempts: vec![Attempt {
            request_sequence: 3,
            ..Default::default()
        }],
        ..Default::default()
    });
    history.turns[0].end = Some(End::Complete);
    history.projection.failed = true;
    history.projection.failed_attempts = vec![Attempt {
        request_sequence: 1,
        os_code: Some(10054),
        ..Default::default()
    }];
    history.checkpoint().unwrap();
    let directory = crate::session::scope::Directory::open(&fixture.0).unwrap();
    assert!(recent_network_attempts_in_store(&store, &id, &directory).is_err());
    drop(history);

    let sessions = store.directory("sessions").unwrap();
    let name = format!("{id}.json");
    let value = crate::json::parse(
        &sessions.read(&name, LIMIT).unwrap().unwrap(),
        Default::default(),
    )
    .unwrap();
    let mut value = value;
    let crate::json::Value::Object(fields) = &mut value else {
        unreachable!()
    };
    let crate::json::Value::Object(projection) = fields.get_mut("projection").unwrap() else {
        unreachable!()
    };
    projection.remove("failed_at_turn");
    let old = crate::json::encode(&value, LIMIT).unwrap();
    sessions.replace(&name, &old).unwrap();

    for _ in 0..2 {
        let attempts = recent_network_attempts_in_store(&store, &id, &directory).unwrap();
        assert_eq!(attempts.len(), 2);
        assert_eq!(attempts[0].source, AttemptSource::Generation);
        assert_eq!(attempts[0].attempt.request_sequence, 3);
        assert_eq!(attempts[1].source, AttemptSource::Compaction);
        assert_eq!(attempts[1].attempt.os_code, Some(10054));
        assert_eq!(sessions.read(&name, LIMIT).unwrap().unwrap(), old);
    }
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
            image: None,
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
            request_elapsed_ms: 900,
            since_progress_ms: Some(120),
            termination: None,
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
