use super::*;
use crate::tls::{ContentType, IoOperation, Plaintext};
use std::{
    collections::VecDeque,
    io,
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};

struct Fixture {
    reads: VecDeque<Result<Vec<u8>, NetworkError>>,
    wire_reads: VecDeque<usize>,
    write_error: Option<NetworkError>,
    close_error: Option<NetworkError>,
    writes: usize,
    closes: usize,
    write_accepted: Option<usize>,
}
impl Fixture {
    fn new() -> Self {
        Self {
            reads: VecDeque::new(),
            wire_reads: VecDeque::new(),
            write_error: None,
            close_error: None,
            writes: 0,
            closes: 0,
            write_accepted: None,
        }
    }
}
impl ResponseChannel for Fixture {
    fn write(
        &mut self,
        bytes: &[u8],
        _: &Budget<'_>,
        progress: &mut ApplicationWrite,
    ) -> Result<(), NetworkError> {
        self.writes += 1;
        progress.accepted_wire_bytes =
            self.write_accepted
                .unwrap_or(if self.write_error.is_some() {
                    0
                } else {
                    bytes.len()
                });
        self.write_error.map_or(Ok(()), Err)
    }
    fn read(&mut self, _: &Budget<'_>) -> Result<Option<Plaintext>, NetworkError> {
        self.reads.pop_front().transpose().map(|bytes| {
            bytes.map(|bytes| Plaintext {
                kind: ContentType::Application,
                bytes,
            })
        })
    }
    fn read_observed(
        &mut self,
        budget: &Budget<'_>,
        received_wire_bytes: &mut usize,
    ) -> Result<Option<Plaintext>, NetworkError> {
        *received_wire_bytes += self.wire_reads.pop_front().unwrap_or(0);
        self.read(budget)
    }
    fn close(&mut self, _: &Budget<'_>) -> Result<(), NetworkError> {
        self.closes += 1;
        self.close_error.map_or(Ok(()), Err)
    }
}
fn budget(cancelled: &AtomicBool) -> Budget<'_> {
    Budget {
        deadline: Some(Instant::now() + Duration::from_secs(5)),
        cancelled,
    }
}
fn http(body: &str) -> Vec<u8> {
    format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    )
    .into_bytes()
}
fn client() -> Client {
    let saved = r#"{"version":1,"state":"ready","provider":"openai-account","access_token":"e30.eyJodHRwczovL2FwaS5vcGVuYWkuY29tL2F1dGgiOnsiY2hhdGdwdF9hY2NvdW50X2lkIjoiYWNjb3VudC10ZXN0In19.c2ln","refresh_token":"synthetic","account_id":"account-test","expires_at":9999999999,"generation":"a1"}"#;
    Client {
        trust: TrustStore::native().unwrap(),
        tokens: auth::Tokens::from_saved_json(saved).unwrap().unwrap(),
        store: None,
        catalog: None,
    }
}
fn request() -> Request {
    Request {
        model: "fixture-model".into(),
        effort: None,
        instructions: "synthetic fixture".into(),
        input: vec![super::super::Input::User("hello".into())],
        tools: Vec::new(),
    }
}
fn complete() -> Fixture {
    let mut channel = Fixture::new();
    channel.reads.push_back(Ok(http("data: {\"type\":\"response.completed\",\"response\":{\"id\":\"r1\",\"status\":\"completed\",\"output\":[]}}\n\n")));
    channel
}
fn reset(operation: IoOperation) -> NetworkError {
    NetworkError::io(operation, &io::Error::from(io::ErrorKind::ConnectionReset))
}

#[test]
fn response_read_failure_keeps_delivered_text_and_safe_error_classification() {
    let failure = NetworkError::io(
        IoOperation::ReadRecordBody,
        &io::Error::new(io::ErrorKind::ConnectionReset, "synthetic secret body"),
    );
    let mut channel = Fixture::new();
    let partial = "data: {\"type\":\"response.output_text.delta\",\"delta\":\"partial\"}\n\n";
    channel.reads.push_back(Ok(format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\n\r\n{partial}",
        partial.len() + 100
    )
    .into_bytes()));
    channel.reads.push_back(Err(failure));
    let cancelled = AtomicBool::new(false);
    let budget = budget(&cancelled);
    let mut shown = String::new();
    let mut delivery = Delivery::NotSubmitted;
    let mut write = ApplicationWrite::default();
    let mut trace = recovery::Trace::default();
    let error = exchange(
        &mut channel,
        b"fixture request",
        &budget,
        &budget,
        ExchangeProgress {
            delivery: &mut delivery,
            write: &mut write,
            trace: &mut trace,
        },
        |progress| {
            if let Progress::Text(text) = progress {
                shown.push_str(text);
            }
            ControlFlow::Continue(())
        },
    )
    .unwrap_err();
    assert_eq!(shown, "partial");
    assert_eq!(channel.writes, 1);
    assert_eq!(channel.closes, 0);
    assert_eq!(
        error,
        Error::Transport {
            stage: RequestStage::ResponseRead,
            error: failure,
            delivery: Delivery::Streaming,
            accepted_wire_bytes: b"fixture request".len(),
        }
    );
    let display = error.to_string();
    assert!(display.contains("response read"));
    assert!(display.contains("TLS record body read"));
    assert!(display.contains("ConnectionReset"));
    assert!(!display.contains("synthetic secret body"));
}

#[test]
fn request_write_failure_is_identified_and_never_replayed() {
    let failure = NetworkError::io(
        IoOperation::WriteRecord,
        &io::Error::from(io::ErrorKind::BrokenPipe),
    );
    let mut channel = Fixture::new();
    channel.write_error = Some(failure);
    let cancelled = AtomicBool::new(false);
    let budget = budget(&cancelled);
    let mut delivery = Delivery::NotSubmitted;
    let mut write = ApplicationWrite::default();
    let mut trace = recovery::Trace::default();
    let error = exchange(
        &mut channel,
        b"fixture request",
        &budget,
        &budget,
        ExchangeProgress {
            delivery: &mut delivery,
            write: &mut write,
            trace: &mut trace,
        },
        |_| panic!("no progress after failed write"),
    )
    .unwrap_err();
    assert_eq!(
        error,
        Error::Transport {
            stage: RequestStage::RequestWrite,
            error: failure,
            delivery: Delivery::NotSubmitted,
            accepted_wire_bytes: 0,
        }
    );
    assert_eq!(channel.writes, 1);
    assert_eq!(channel.closes, 0);
}

#[test]
fn validated_completion_survives_close_failure_without_another_read() {
    let body = "data: {\"type\":\"response.output_text.delta\",\"delta\":\"finished\"}\n\ndata: {\"type\":\"response.completed\",\"response\":{\"id\":\"r1\",\"status\":\"completed\",\"output\":[{\"type\":\"message\",\"content\":[{\"type\":\"output_text\",\"text\":\"finished\"}]}]}}\n\n";
    let mut channel = Fixture::new();
    channel.reads.push_back(Ok(http(body)));
    channel
        .reads
        .push_back(Err(reset(IoOperation::ReadRecordBody)));
    channel.close_error = Some(NetworkError::io(
        IoOperation::WriteRecord,
        &io::Error::from(io::ErrorKind::BrokenPipe),
    ));
    let cancelled = AtomicBool::new(false);
    let budget = budget(&cancelled);
    let mut shown = String::new();
    let mut delivery = Delivery::NotSubmitted;
    let mut write = ApplicationWrite::default();
    let mut trace = recovery::Trace::default();
    let response = exchange(
        &mut channel,
        b"fixture request",
        &budget,
        &budget,
        ExchangeProgress {
            delivery: &mut delivery,
            write: &mut write,
            trace: &mut trace,
        },
        |progress| {
            if let Progress::Text(text) = progress {
                shown.push_str(text);
            }
            ControlFlow::Continue(())
        },
    )
    .unwrap();
    assert_eq!(response.text, "finished");
    assert_eq!(shown, "finished");
    assert_eq!(response.usage.input, None);
    assert_eq!(channel.writes, 1);
    assert_eq!(channel.closes, 1);
    assert_eq!(channel.reads.len(), 1);
    assert_eq!(delivery, Delivery::Completed);
}

#[test]
fn interrupted_body_read_records_bounded_response_progress_without_content() {
    let secret = "synthetic prompt and response body secret";
    let mut channel = Fixture::new();
    let partial =
        format!("data: {{\"type\":\"response.output_text.delta\",\"delta\":\"{secret}\"}}\n\n");
    let first = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\n\r\n{partial}",
        partial.len() + 100
    )
    .into_bytes();
    channel.wire_reads.extend([first.len() + 22, 7]);
    channel.reads.push_back(Ok(first.clone()));
    channel.reads.push_back(Err(NetworkError::io(
        IoOperation::ReadRecordBody,
        &io::Error::new(io::ErrorKind::ConnectionReset, secret),
    )));
    let cancelled = AtomicBool::new(false);
    let mut attempts = Vec::new();
    let mut shown = String::new();
    let error = client()
        .generate_with(
            &request(),
            &budget(&cancelled),
            |progress| {
                match progress {
                    Progress::Text(text) => shown.push_str(text),
                    Progress::Attempt(attempt) => attempts.push(attempt),
                    Progress::Reasoning(_) => {}
                }
                ControlFlow::Continue(())
            },
            |_, _| Ok(std::mem::replace(&mut channel, Fixture::new())),
        )
        .unwrap_err();
    assert_eq!(shown, secret);
    assert_eq!(attempts.len(), 1);
    let attempt = &attempts[0];
    assert_eq!(attempt.connection_attempt, 1);
    assert_eq!(attempt.stage, Some(RequestStage::ResponseRead));
    assert_eq!(attempt.delivery, Delivery::Streaming);
    assert_eq!(attempt.received_wire_bytes, first.len() + 29);
    assert_eq!(attempt.response_plaintext_bytes, first.len());
    assert_eq!(attempt.response_status, Some(200));
    assert_eq!(attempt.stream_events, 1);
    assert!(attempt.accepted_wire_bytes > 0);
    assert!(!attempt.retrying);
    assert!(!format!("{attempt:?} {error}").contains(secret));
}

#[test]
fn timeout_after_a_started_stream_keeps_partial_evidence_and_never_resubmits() {
    let mut channel = Fixture::new();
    let partial = "data: {\"type\":\"response.output_text.delta\",\"delta\":\"partial\"}\n\n";
    channel.reads.push_back(Ok(format!(
        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{partial}",
        partial.len() + 100
    )
    .into_bytes()));
    channel.reads.push_back(Err(NetworkError::Timeout));
    let cancelled = AtomicBool::new(false);
    let parent = Budget {
        deadline: None,
        cancelled: &cancelled,
    };
    let mut connections = 0;
    let mut attempts = Vec::new();
    let mut shown = String::new();
    let error = client()
        .generate_with(
            &request(),
            &parent,
            |progress| {
                match progress {
                    Progress::Text(text) => shown.push_str(text),
                    Progress::Attempt(attempt) => attempts.push(attempt),
                    Progress::Reasoning(_) => {}
                }
                ControlFlow::Continue(())
            },
            |_, _| {
                connections += 1;
                Ok(std::mem::replace(&mut channel, Fixture::new()))
            },
        )
        .unwrap_err();
    assert!(matches!(
        error,
        Error::Transport {
            error: NetworkError::Timeout,
            delivery: Delivery::Streaming,
            ..
        }
    ));
    assert_eq!(connections, 1);
    assert_eq!(shown, "partial");
    assert_eq!(attempts.len(), 1);
    assert_eq!(attempts[0].termination, Some(Termination::IdleTimeout));
    assert_eq!(attempts[0].stream_events, 1);
    assert!(!attempts[0].retrying);
}

#[test]
fn os_code_is_structured_without_exposing_io_message() {
    let failure = crate::tls::IoFailure {
        operation: IoOperation::Connect,
        kind: io::ErrorKind::ConnectionRefused,
        os_code: Some(12345),
    };
    let display = NetworkError::Io(failure).to_string();
    assert!(display.contains("TCP connect"));
    assert!(display.contains("ConnectionRefused"));
    assert!(display.contains("OS 12345"));
}

#[path = "timing_tests.rs"]
mod timing_tests;

#[path = "recovery_tests.rs"]
mod recovery_tests;
