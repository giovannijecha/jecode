use super::*;
use crate::providers::openai_account::{FailureCode, FailureEvent, ProviderFailure};

#[test]
fn provider_failure_attempt_keeps_event_code_and_stream_delivery_without_retry() {
    let sentinel = "synthetic-secret-token";
    for (payload, event, code) in [
        (
            r#"{"type":"response.failed","response":{"id":"r1","status":"failed","error":{"code":"server_error","message":"synthetic-secret-token"}}}"#,
            FailureEvent::ResponseFailed,
            FailureCode::ServerError,
        ),
        (
            r#"{"type":"error","code":"rate_limit_exceeded","message":"synthetic-secret-token"}"#,
            FailureEvent::Error,
            FailureCode::RateLimitExceeded,
        ),
    ] {
        let cancelled = AtomicBool::new(false);
        let body = format!("data: {payload}\n\n");
        let mut channel = Fixture::new();
        channel.reads.push_back(Ok(http(&body)));
        let mut connections = 0;
        let mut attempts = Vec::new();
        let error = client()
            .generate_with(
                &request(),
                &budget(&cancelled),
                |progress| {
                    if let Progress::Attempt(attempt) = progress {
                        attempts.push(attempt);
                    }
                    ControlFlow::Continue(())
                },
                |_, _| {
                    connections += 1;
                    Ok(std::mem::replace(&mut channel, Fixture::new()))
                },
            )
            .unwrap_err();
        let failure = ProviderFailure { event, code };
        assert_eq!(
            error,
            Error::Response {
                error: crate::providers::openai_account::Error::RemoteFailure(failure),
                delivery: Delivery::Streaming,
            }
        );
        assert_eq!(connections, 1);
        assert_eq!(attempts.len(), 1);
        let attempt = &attempts[0];
        assert_eq!(attempt.provider_failure, Some(failure));
        assert_eq!(attempt.delivery, Delivery::Streaming);
        assert_eq!(attempt.stage, Some(RequestStage::ResponseRead));
        assert_eq!(attempt.response_status, Some(200));
        assert_eq!(attempt.stream_events, 1);
        assert!(attempt.accepted_wire_bytes > 0);
        assert!(!attempt.retrying);
        assert!(!format!("{attempt:?} {error}").contains(sentinel));
    }
}

#[test]
fn pre_submission_connection_failure_retries_once_then_completes() {
    let cancelled = AtomicBool::new(false);
    let mut client = client();
    let mut connections = 0;
    let mut attempts = Vec::new();
    let response = client
        .generate_with(
            &request(),
            &budget(&cancelled),
            |progress| {
                if let Progress::Attempt(attempt) = progress {
                    attempts.push(attempt);
                }
                ControlFlow::Continue(())
            },
            |_, _| {
                connections += 1;
                if connections == 1 {
                    Err(reset(IoOperation::Connect))
                } else {
                    Ok(complete())
                }
            },
        )
        .unwrap();
    assert_eq!(connections, 2);
    assert_eq!(attempts.len(), 2);
    assert_eq!(
        (
            attempts[0].delivery,
            attempts[0].retrying,
            attempts[0].accepted_wire_bytes
        ),
        (Delivery::NotSubmitted, true, 0)
    );
    assert_eq!(attempts[1].delivery, Delivery::Completed);
    assert_eq!(
        response.status,
        crate::providers::openai_account::Status::Completed
    );
}

#[test]
fn bounded_recovery_exhausts_exactly_three_connections() {
    let cancelled = AtomicBool::new(false);
    let mut client = client();
    let mut connections = 0;
    let mut attempts = Vec::new();
    let error = client
        .generate_with::<Fixture>(
            &request(),
            &budget(&cancelled),
            |progress| {
                if let Progress::Attempt(attempt) = progress {
                    attempts.push(attempt);
                }
                ControlFlow::Continue(())
            },
            |_, _| {
                connections += 1;
                Err(reset(IoOperation::Connect))
            },
        )
        .unwrap_err();
    assert_eq!(connections, 3);
    assert_eq!(attempts.len(), 3);
    assert_eq!(
        attempts.iter().filter(|attempt| attempt.retrying).count(),
        2
    );
    assert!(
        attempts
            .iter()
            .all(|attempt| attempt.delivery == Delivery::NotSubmitted)
    );
    assert!(matches!(
        error,
        Error::Transport {
            stage: RequestStage::Connect,
            delivery: Delivery::NotSubmitted,
            ..
        }
    ));
}

#[test]
fn cancellation_during_backoff_does_not_open_a_second_connection() {
    let cancelled = AtomicBool::new(false);
    let mut client = client();
    let mut connections = 0;
    let result = client.generate_with::<Fixture>(
        &request(),
        &budget(&cancelled),
        |progress| {
            if let Progress::Attempt(attempt) = progress {
                assert!(attempt.retrying);
                cancelled.store(true, Ordering::Release);
            }
            ControlFlow::Continue(())
        },
        |_, _| {
            connections += 1;
            Err(reset(IoOperation::Connect))
        },
    );
    assert_eq!(result, Err(Error::Network(NetworkError::Cancelled)));
    assert_eq!(connections, 1);
}

#[test]
fn cancellation_during_connection_setup_never_sends_a_request() {
    let cancelled = AtomicBool::new(false);
    let mut client = client();
    let mut connections = 0;
    let mut attempts = Vec::new();
    let result = client.generate_with::<Fixture>(
        &request(),
        &budget(&cancelled),
        |progress| {
            if let Progress::Attempt(attempt) = progress {
                attempts.push(attempt);
            }
            ControlFlow::Continue(())
        },
        |_, _| {
            connections += 1;
            cancelled.store(true, Ordering::Release);
            Err(NetworkError::Cancelled)
        },
    );
    assert_eq!(
        result,
        Err(Error::Transport {
            stage: RequestStage::Connect,
            error: NetworkError::Cancelled,
            delivery: Delivery::NotSubmitted,
            accepted_wire_bytes: 0,
        })
    );
    assert_eq!(connections, 1);
    assert_eq!(attempts.len(), 1);
    assert_eq!(attempts[0].connection_attempt, 1);
    assert!(!attempts[0].retrying);
}

#[test]
fn zero_progress_write_retries_but_partial_write_does_not() {
    for accepted in [0, 3] {
        let cancelled = AtomicBool::new(false);
        let mut client = client();
        let mut connections = 0;
        let mut attempts = Vec::new();
        let result = client.generate_with(
            &request(),
            &budget(&cancelled),
            |progress| {
                if let Progress::Attempt(attempt) = progress {
                    attempts.push(attempt);
                }
                ControlFlow::Continue(())
            },
            |_, _| {
                connections += 1;
                if connections > 1 {
                    return Ok(complete());
                }
                let mut channel = Fixture::new();
                channel.write_error = Some(reset(IoOperation::WriteRecord));
                channel.write_accepted = Some(accepted);
                Ok(channel)
            },
        );
        if accepted == 0 {
            assert!(result.is_ok());
            assert_eq!(connections, 2);
            assert_eq!(attempts[0].delivery, Delivery::NotSubmitted);
        } else {
            assert!(matches!(
                result,
                Err(Error::Transport {
                    delivery: Delivery::PossiblySubmitted,
                    accepted_wire_bytes: 3,
                    ..
                })
            ));
            assert_eq!(connections, 1);
            assert_eq!(attempts[0].delivery, Delivery::PossiblySubmitted);
            assert!(!attempts[0].retrying);
        }
    }
}

#[test]
fn reset_after_request_and_partial_stream_never_replay() {
    for partial in [false, true] {
        let cancelled = AtomicBool::new(false);
        let mut client = client();
        let mut connections = 0;
        let mut shown = String::new();
        let mut attempts = Vec::new();
        let error = client.generate_with(&request(), &budget(&cancelled), |progress| {
            match progress {
                Progress::Text(text) => shown.push_str(text),
                Progress::Attempt(attempt) => attempts.push(attempt),
                Progress::Reasoning(_) => {}
            }
            ControlFlow::Continue(())
        }, |_, _| {
            connections += 1;
            let mut channel = Fixture::new();
            if partial {
                let body = "data: {\"type\":\"response.output_text.delta\",\"delta\":\"partial\"}\n\n";
                channel.reads.push_back(Ok(format!("HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\n\r\n{body}", body.len() + 100).into_bytes()));
            }
            channel.reads.push_back(Err(reset(IoOperation::ReadRecordHeader)));
            Ok(channel)
        }).unwrap_err();
        assert_eq!(connections, 1);
        assert_eq!(attempts.len(), 1);
        assert_eq!(
            attempts[0].delivery,
            if partial {
                Delivery::Streaming
            } else {
                Delivery::PossiblySubmitted
            }
        );
        assert_eq!(shown, if partial { "partial" } else { "" });
        assert!(!attempts[0].retrying);
        assert!(error.to_string().contains("remote outcome uncertain"));
    }
}

#[test]
fn protocol_http_alert_and_certificate_failures_do_not_retry() {
    let cancelled = AtomicBool::new(false);
    let mut client = client();
    let mut connections = 0;
    let error = client
        .generate_with::<Fixture>(
            &request(),
            &budget(&cancelled),
            |_| ControlFlow::Continue(()),
            |_, _| {
                connections += 1;
                Err(NetworkError::Certificate(
                    crate::tls::certificate::Error::Name,
                ))
            },
        )
        .unwrap_err();
    assert!(matches!(
        error,
        Error::Transport {
            stage: RequestStage::Connect,
            delivery: Delivery::NotSubmitted,
            ..
        }
    ));
    assert_eq!(connections, 1);

    for (channel, expected, delivery) in [
        (
            {
                let mut channel = Fixture::new();
                channel.reads.push_back(Ok(
                    b"HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\n\r\n".to_vec(),
                ));
                channel
            },
            "HTTP 401",
            Delivery::PossiblySubmitted,
        ),
        (
            {
                let mut channel = Fixture::new();
                channel.reads.push_back(Ok(http("data: invalid-json\n\n")));
                channel
            },
            "invalid",
            Delivery::Streaming,
        ),
        (
            {
                let mut channel = Fixture::new();
                channel
                    .reads
                    .push_back(Err(NetworkError::Tls(crate::tls::Error::PeerAlert)));
                channel
            },
            "secure transport",
            Delivery::PossiblySubmitted,
        ),
        (
            {
                let mut channel = Fixture::new();
                channel.reads.push_back(Ok(Vec::new()));
                channel
            },
            "invalid HTTP framing",
            Delivery::PossiblySubmitted,
        ),
    ] {
        let mut channel = Some(channel);
        let mut attempts = Vec::new();
        let error = client
            .generate_with(
                &request(),
                &budget(&cancelled),
                |progress| {
                    if let Progress::Attempt(attempt) = progress {
                        attempts.push(attempt);
                    }
                    ControlFlow::Continue(())
                },
                |_, _| {
                    connections += 1;
                    Ok(channel.take().unwrap())
                },
            )
            .unwrap_err();
        assert_eq!(attempts.len(), 1);
        assert_eq!(attempts[0].delivery, delivery);
        assert!(!attempts[0].retrying);
        assert!(error.to_string().contains(expected), "{error}");
    }
    assert_eq!(connections, 5);
}
