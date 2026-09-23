use super::*;
use crate::tls::{ContentType, IoOperation};
use std::{collections::VecDeque, io, sync::atomic::AtomicBool, time::Instant};

struct Fixture {
    reads: VecDeque<Result<Vec<u8>, NetworkError>>,
    write_error: Option<NetworkError>,
    close_error: Option<NetworkError>,
    writes: usize,
    closes: usize,
}
impl Fixture {
    fn new() -> Self {
        Self {
            reads: VecDeque::new(),
            write_error: None,
            close_error: None,
            writes: 0,
            closes: 0,
        }
    }
}
impl ResponseChannel for Fixture {
    fn write(&mut self, _: &[u8], _: &Budget<'_>) -> Result<(), NetworkError> {
        self.writes += 1;
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
    let error = exchange(
        &mut channel,
        b"fixture request",
        &budget,
        &budget,
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
            error: failure
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
    let error = exchange(&mut channel, b"fixture request", &budget, &budget, |_| {
        panic!("no progress after failed write")
    })
    .unwrap_err();
    assert_eq!(
        error,
        Error::Transport {
            stage: RequestStage::RequestWrite,
            error: failure
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
    let response = exchange(
        &mut channel,
        b"fixture request",
        &budget,
        &budget,
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
