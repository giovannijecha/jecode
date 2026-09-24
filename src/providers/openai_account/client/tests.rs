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
    fn close(&mut self, _: &Budget<'_>) -> Result<(), NetworkError> {
        self.closes += 1;
        self.close_error.map_or(Ok(()), Err)
    }
}
fn budget(cancelled: &AtomicBool) -> Budget<'_> {
    Budget {
        deadline: Instant::now() + Duration::from_secs(5),
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
    let error = exchange(
        &mut channel,
        b"fixture request",
        &budget,
        &budget,
        &mut delivery,
        &mut write,
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
    let error = exchange(
        &mut channel,
        b"fixture request",
        &budget,
        &budget,
        &mut delivery,
        &mut write,
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
    channel.close_error = Some(NetworkError::io(
        IoOperation::WriteRecord,
        &io::Error::from(io::ErrorKind::BrokenPipe),
    ));
    let cancelled = AtomicBool::new(false);
    let budget = budget(&cancelled);
    let mut shown = String::new();
    let mut delivery = Delivery::NotSubmitted;
    let mut write = ApplicationWrite::default();
    let response = exchange(
        &mut channel,
        b"fixture request",
        &budget,
        &budget,
        &mut delivery,
        &mut write,
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
    assert_eq!(delivery, Delivery::Completed);
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

#[path = "recovery_tests.rs"]
mod recovery_tests;
