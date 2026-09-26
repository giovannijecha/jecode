use super::*;
use crate::{
    providers::openai_account::{
        Error as ProviderError, FailureCode, FailureEvent, ProviderFailure,
        client::{Attempt, Delivery, RequestStage},
    },
    session::{
        End, Failure,
        history::{Receipt, Step},
    },
};

fn failure(event: FailureEvent, code: FailureCode) -> ProviderFailure {
    ProviderFailure { event, code }
}

fn attempt(provider_failure: ProviderFailure, sequence: u32) -> Attempt {
    Attempt {
        request_sequence: sequence,
        connection_attempt: 1,
        delivery: Delivery::Streaming,
        stage: Some(RequestStage::ResponseRead),
        stage_elapsed_ms: 81,
        request_elapsed_ms: 99,
        since_progress_ms: Some(3),
        accepted_wire_bytes: 511,
        received_wire_bytes: 704,
        response_plaintext_bytes: 682,
        response_status: Some(200),
        stream_events: 2,
        provider_failure: Some(provider_failure),
        diagnostic: Some(ProviderError::RemoteFailure(provider_failure).to_string()),
        ..Default::default()
    }
}

#[test]
fn failure_metadata_survives_generation_compaction_resume_and_read_only_export() {
    let generation = failure(FailureEvent::ResponseFailed, FailureCode::ServerError);
    let compaction = failure(FailureEvent::Error, FailureCode::RateLimitExceeded);
    for legacy in [false, true] {
        for manual in [false, true] {
            let fixture = crate::state::tests::Fixture::new();
            let Some(store) = fixture.store() else {
                continue;
            };
            let mut history = if legacy {
                create_legacy_in(&store, Model::Luna, Some(&fixture.0), None).unwrap()
            } else {
                create_in(&store, Model::Luna, Some(&fixture.0), None).unwrap()
            };
            let id = history.record.as_ref().unwrap().id().to_owned();
            history.begin("synthetic task".into()).unwrap();
            history.turns[0].steps.push(Step {
                response: Some(crate::session::tool_tests::calls_response(vec![
                    crate::session::tool_tests::call(
                        "read-1",
                        "read_file",
                        r#"{"path":"notes.txt"}"#,
                    ),
                ])),
                accepted: true,
                results: vec![Receipt {
                    call_id: "read-1".into(),
                    output: "completed synthetic receipt".into(),
                    summary: "Read notes.txt".into(),
                    image: None,
                }],
                ..Default::default()
            });
            history.turns[0].steps.push(Step {
                attempts: vec![attempt(generation, 2)],
                ..Default::default()
            });
            if manual {
                history.turns[0].end = Some(End::Complete);
                history.checkpoint().unwrap();
            }
            assert_eq!(
                crate::session::context::failed_attempt(
                    &mut history,
                    Failure::Worker,
                    vec![attempt(compaction, 1)],
                    String::new(),
                ),
                Failure::Worker
            );
            if !manual {
                history.turns[0].end = Some(End::Failed(Failure::Worker));
                history.checkpoint().unwrap();
            }
            drop(history);

            let sessions = store
                .directory(if legacy { "sessions" } else { "sessions-v2" })
                .unwrap();
            let names = if legacy {
                vec![format!("{id}.json")]
            } else {
                vec![format!("{id}.head"), format!("{id}.log")]
            };
            let before: Vec<_> = names
                .iter()
                .map(|name| std::fs::read(sessions.root().join(name)).unwrap())
                .collect();
            assert!(before.iter().all(|bytes| !String::from_utf8_lossy(bytes).contains("synthetic-secret-token")));
            let directory = crate::session::scope::Directory::open(&fixture.0).unwrap();
            for _ in 0..2 {
                let exported = recent_network_attempts_in_store(&store, &id, &directory).unwrap();
                assert_eq!(exported.len(), 2);
                assert_eq!(exported[0].source, AttemptSource::Generation);
                assert_eq!(exported[0].attempt.provider_failure, Some(generation));
                assert_eq!(exported[1].source, AttemptSource::Compaction);
                assert_eq!(exported[1].attempt.provider_failure, Some(compaction));
                assert!(exported.iter().all(|entry| {
                    entry.attempt.delivery == Delivery::Streaming
                        && entry.attempt.stage == Some(RequestStage::ResponseRead)
                        && entry.attempt.response_status == Some(200)
                        && !entry.attempt.retrying
                }));
                for (name, saved) in names.iter().zip(&before) {
                    assert_eq!(&std::fs::read(sessions.root().join(name)).unwrap(), saved);
                }
            }
            let resumed = resume_in_store(&store, &id, &directory).unwrap();
            let steps = &resumed.history.turns[0].steps;
            assert_eq!(steps[0].results[0].output, "completed synthetic receipt");
            assert_eq!(steps[1].attempts[0].provider_failure, Some(generation));
            assert!(steps[1].response.is_none());
            assert_eq!(
                resumed.history.projection.failed_attempts[0].provider_failure,
                Some(compaction)
            );
            drop(resumed);
        }
    }
}

#[test]
fn legacy_attempt_metadata_is_unknown_and_invalid_saved_labels_are_rejected() {
    let old = crate::json::parse(
        r#"{"delivery":"streaming_unvalidated","accepted_wire_bytes":12,"retrying":false}"#,
        Default::default(),
    )
    .unwrap();
    let decoded = codec::read_attempt(&old).unwrap();
    assert_eq!(decoded.provider_failure, None);
    for (event, code) in [
        ("secret-event", "server_error"),
        ("error", "secret-code"),
        (
            "error",
            "xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx",
        ),
    ] {
        let mut value = codec::attempt(&decoded);
        let Value::Object(fields) = &mut value else {
            unreachable!()
        };
        fields.insert(
            "provider_failure".into(),
            json::object([("event", text(event)), ("code", text(code))]),
        );
        assert!(codec::read_attempt(&value).is_err());
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
    let crate::json::Value::Array(turns) = fields.get_mut("history").unwrap() else {
        unreachable!()
    };
    let crate::json::Value::Object(turn) = &mut turns[0] else {
        unreachable!()
    };
    let crate::json::Value::Array(steps) = turn.get_mut("steps").unwrap() else {
        unreachable!()
    };
    let crate::json::Value::Object(step) = &mut steps[0] else {
        unreachable!()
    };
    let crate::json::Value::Array(attempts) = step.get_mut("attempts").unwrap() else {
        unreachable!()
    };
    let crate::json::Value::Object(generation) = &mut attempts[0] else {
        unreachable!()
    };
    generation.remove("provider_failure");
    let crate::json::Value::Object(projection) = fields.get_mut("projection").unwrap() else {
        unreachable!()
    };
    projection.remove("failed_at_turn");
    let crate::json::Value::Array(attempts) = projection.get_mut("failed_attempts").unwrap() else {
        unreachable!()
    };
    let crate::json::Value::Object(compaction) = &mut attempts[0] else {
        unreachable!()
    };
    compaction.remove("provider_failure");
    let old = crate::json::encode(&value, LIMIT).unwrap();
    sessions.replace(&name, &old).unwrap();

    for _ in 0..2 {
        let attempts = recent_network_attempts_in_store(&store, &id, &directory).unwrap();
        assert_eq!(attempts.len(), 2);
        assert_eq!(attempts[0].source, AttemptSource::Generation);
        assert_eq!(attempts[0].attempt.request_sequence, 3);
        assert_eq!(attempts[0].attempt.provider_failure, None);
        assert_eq!(attempts[1].source, AttemptSource::Compaction);
        assert_eq!(attempts[1].attempt.os_code, Some(10054));
        assert_eq!(attempts[1].attempt.provider_failure, None);
        assert_eq!(sessions.read(&name, LIMIT).unwrap().unwrap(), old);
    }
}
