use super::*;
use crate::backend::FailureKind;
use crate::http::adapter::{Adapter, Frame};
use crate::test_support::server::{self, Server};
use std::io::{Read, Write};
use std::sync::Arc;
use std::time::Duration;

mod fixture;
use fixture::{Peer, header, reply};

fn backend(endpoint: &str) -> OpenAi {
    let api = Api::fixture(Session::fixture("fixture-access-token"), endpoint).with_sockets();
    let mut backend = OpenAi::with_api(api, "gpt-fixture".into()).unwrap();
    backend.set_conversation(Some("conversation-1".into()));
    backend
}

fn sample(backend: &OpenAi, prompt: &str) -> Result<backend::Completion, backend::Failure> {
    let messages = [Value::object([
        ("role", Value::string("user")),
        ("content", Value::string(prompt)),
    ])];
    backend.sample(&messages, false, None, None, &mut |_| Ok(()))
}

fn input_text(event: &Value) -> String {
    event.get("input").unwrap().encode()
}

#[test]
fn a_conversation_reuses_one_connection_and_sends_full_requests() {
    let server = Server::start(|listener, stop| {
        let mut peer = Peer::new(server::accept(&listener, &stop).unwrap());
        let headers = peer.upgrade();
        let first = peer.message().unwrap();
        peer.answer("One");
        let second = peer.message().unwrap();
        peer.answer("Two");
        (headers, first, second)
    });
    let mut backend = backend(&server.endpoint);
    assert_eq!(sample(&backend, "first prompt").unwrap().text, "One");
    backend.set_conversation(Some("conversation-1".into()));
    assert_eq!(sample(&backend, "second prompt").unwrap().text, "Two");
    let (headers, first, second) = server.finish();
    assert_eq!(
        header(&headers, "authorization"),
        Some("Bearer fixture-access-token")
    );
    assert_eq!(
        header(&headers, "openai-beta"),
        Some("responses_websockets=2026-02-06")
    );
    assert_eq!(header(&headers, "session-id"), Some("conversation-1"));
    for event in [&first, &second] {
        assert_eq!(
            event.get("type").and_then(Value::as_str),
            Some("response.create")
        );
        assert_eq!(event.get("stream"), None);
        assert_eq!(event.get("store"), Some(&Value::Bool(false)));
        assert_eq!(event.get("previous_response_id"), None);
        assert_eq!(
            event.get("prompt_cache_key").and_then(Value::as_str),
            Some("conversation-1")
        );
    }
    assert!(input_text(&first).contains("first prompt"));
    assert!(input_text(&second).contains("second prompt"));
}

#[test]
fn changing_conversation_closes_the_old_socket_and_uses_the_new_session_header() {
    let server = Server::start(|listener, stop| {
        let mut first = Peer::new(server::accept(&listener, &stop).unwrap());
        let first_headers = first.upgrade();
        first.message().unwrap();
        first.answer("One");
        let old_closed = first.message().is_none();
        if !old_closed {
            first.answer("Wrong conversation");
            return (first_headers, None, None, old_closed);
        }
        let mut second = Peer::new(server::accept(&listener, &stop).unwrap());
        let second_headers = second.upgrade();
        let event = second.message().unwrap();
        second.answer("Two");
        (first_headers, Some(second_headers), Some(event), old_closed)
    });
    let mut backend = backend(&server.endpoint);
    assert_eq!(sample(&backend, "first").unwrap().text, "One");
    backend.set_conversation(Some("conversation-2".into()));
    let second = sample(&backend, "second").unwrap();
    let (first_headers, second_headers, event, old_closed) = server.finish();
    assert!(old_closed);
    assert_eq!(header(&first_headers, "session-id"), Some("conversation-1"));
    assert_eq!(
        header(&second_headers.unwrap(), "session-id"),
        Some("conversation-2")
    );
    assert_eq!(
        event
            .unwrap()
            .get("prompt_cache_key")
            .and_then(Value::as_str),
        Some("conversation-2")
    );
    assert_eq!(second.text, "Two");
}

#[test]
fn a_replaced_access_token_opens_a_new_connection() {
    let server = Server::start(|listener, stop| {
        let mut first = Peer::new(server::accept(&listener, &stop).unwrap());
        first.upgrade();
        first.message().unwrap();
        first.answer("One");
        let old_closed = first.message().is_none();
        if !old_closed {
            first.answer("Old token");
            return (old_closed, None);
        }
        let mut second = Peer::new(server::accept(&listener, &stop).unwrap());
        let headers = second.upgrade();
        second.message().unwrap();
        second.answer("Two");
        (old_closed, Some(headers))
    });
    let session = Session::fixture("fixture-access-token");
    let api = Api::fixture(session.clone(), &server.endpoint).with_sockets();
    let mut backend = OpenAi::with_api(api, "gpt-fixture".into()).unwrap();
    backend.set_conversation(Some("conversation-1".into()));
    assert_eq!(sample(&backend, "first").unwrap().text, "One");
    session.fixture_replace_token("replaced-access-token");
    assert_eq!(sample(&backend, "second").unwrap().text, "Two");
    let (old_closed, headers) = server.finish();
    assert!(old_closed);
    assert_eq!(
        header(&headers.unwrap(), "authorization"),
        Some("Bearer replaced-access-token")
    );
}

#[test]
fn signing_out_closes_the_connection_and_reports_it_like_http() {
    let server = Server::start(|listener, stop| {
        let mut peer = Peer::new(server::accept(&listener, &stop).unwrap());
        peer.upgrade();
        peer.message().unwrap();
        peer.answer("One");
        let closed = peer.message().is_none();
        if !closed {
            peer.answer("After sign-out");
        }
        // A second connection would mean the signed-out request went elsewhere.
        (closed, server::accept(&listener, &stop).is_err())
    });
    let session = Session::fixture("fixture-access-token");
    let api = Api::fixture(session.clone(), &server.endpoint).with_sockets();
    let mut backend = OpenAi::with_api(api, "gpt-fixture".into()).unwrap();
    backend.set_conversation(Some("conversation-1".into()));
    assert_eq!(sample(&backend, "first").unwrap().text, "One");
    session.fixture_replace_token("");
    let failure = sample(&backend, "second").err().unwrap();
    assert!(
        failure.message.contains("signed out"),
        "{}",
        failure.message
    );
    assert!(matches!(failure.kind, FailureKind::Terminal));
    assert_eq!(server.finish(), (true, true));
}

#[test]
fn dropping_the_backend_closes_its_connection() {
    let server = Server::start(|listener, stop| {
        let mut peer = Peer::new(server::accept(&listener, &stop).unwrap());
        peer.upgrade();
        peer.message().unwrap();
        peer.answer("One");
        // Only the adapter noticing its closed stdin can end this read.
        peer.message().is_none()
    });
    let backend = backend(&server.endpoint);
    assert_eq!(sample(&backend, "first").unwrap().text, "One");
    drop(backend);
    assert!(server.finish());
}

#[test]
fn an_idle_socket_adapter_exits_and_closes_its_connection() {
    let server = Server::start(|listener, stop| {
        let mut peer = Peer::new(server::accept(&listener, &stop).unwrap());
        peer.upgrade();
        peer.message().is_none()
    });
    let url = format!("{}/responses", server.endpoint);
    let mut adapter = Adapter::start(&["socket", "300"]).unwrap();
    let opening = Arc::new(crate::http::socket_opening(&url, &[]));
    let delivery = adapter.deliver(opening).unwrap();
    assert!(matches!(adapter.frames.recv().unwrap(), Frame::Accepted));
    assert!(adapter.recover(delivery));
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while adapter.alive() {
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(10));
    }
    adapter.stop();
    assert!(server.finish());
}

#[test]
fn the_api_key_route_sends_no_session_header() {
    let server = Server::start(|listener, stop| {
        let mut peer = Peer::new(server::accept(&listener, &stop).unwrap());
        let headers = peer.upgrade();
        peer.message().unwrap();
        peer.answer("Done");
        headers
    });
    let api = Api::fixture_api_key("fixture-key", &server.endpoint).with_sockets();
    let mut backend = OpenAi::with_api(api, "gpt-fixture".into()).unwrap();
    backend.set_conversation(Some("conversation-1".into()));
    assert_eq!(sample(&backend, "prompt").unwrap().text, "Done");
    let headers = server.finish();
    assert_eq!(
        header(&headers, "authorization"),
        Some("Bearer fixture-key")
    );
    assert_eq!(header(&headers, "session-id"), None);
}

/// Answers one HTTP Responses request with a complete event stream.
fn answer_http(peer: &mut Peer, text: &str) -> String {
    let (first, headers) = peer.head();
    let length: usize = header(&headers, "content-length").unwrap().parse().unwrap();
    let mut body = vec![0; length];
    peer.reader.read_exact(&mut body).unwrap();
    let events: String = reply(text)
        .iter()
        .map(|event| format!("data: {event}\n\n"))
        .collect();
    write!(
        peer.writer,
        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{events}",
        events.len()
    )
    .unwrap();
    first
}

#[test]
fn a_refused_upgrade_falls_back_to_http_and_pauses_later_attempts() {
    let server = Server::start(|listener, stop| {
        let mut refused = Peer::new(server::accept(&listener, &stop).unwrap());
        let (first, _) = refused.head();
        assert!(first.starts_with("GET "), "{first}");
        refused
            .writer
            .write_all(b"HTTP/1.1 404 Not Found\r\nConnection: close\r\nContent-Length: 0\r\n\r\n")
            .unwrap();
        drop(refused);
        let mut requests = Vec::new();
        for text in ["One", "Two"] {
            let mut peer = Peer::new(server::accept(&listener, &stop).unwrap());
            requests.push(answer_http(&mut peer, text));
        }
        requests
    });
    let backend = backend(&server.endpoint);
    assert_eq!(sample(&backend, "first").unwrap().text, "One");
    assert_eq!(sample(&backend, "second").unwrap().text, "Two");
    for request in server.finish() {
        assert!(request.starts_with("POST /responses "), "{request}");
    }
}

#[test]
fn a_request_without_a_conversation_stays_on_http() {
    let server = Server::start(|listener, stop| {
        let mut peer = Peer::new(server::accept(&listener, &stop).unwrap());
        answer_http(&mut peer, "Titled")
    });
    let api = Api::fixture(Session::fixture("fixture-access-token"), &server.endpoint);
    let backend = OpenAi::with_api(api.with_sockets(), "gpt-fixture".into()).unwrap();
    assert_eq!(sample(&backend, "name this").unwrap().text, "Titled");
    assert!(server.finish().starts_with("POST /responses "));
}

#[test]
fn the_path_taken_is_reported_before_the_response() {
    let server = Server::start(|listener, stop| {
        let mut refused = Peer::new(server::accept(&listener, &stop).unwrap());
        refused.head();
        refused
            .writer
            .write_all(b"HTTP/1.1 404 Not Found\r\nConnection: close\r\nContent-Length: 0\r\n\r\n")
            .unwrap();
        drop(refused);
        let mut peer = Peer::new(server::accept(&listener, &stop).unwrap());
        answer_http(&mut peer, "Over HTTP");
        let mut direct = Peer::new(server::accept(&listener, &stop).unwrap());
        direct.upgrade();
        direct.message().unwrap();
        direct.answer("Over the socket");
    });
    let messages = [Value::object([
        ("role", Value::string("user")),
        ("content", Value::string("prompt")),
    ])];
    let path = |backend: &OpenAi| {
        let mut seen = Vec::new();
        backend
            .sample(&messages, false, None, None, &mut |update| {
                match update {
                    backend::Update::Transport(name) => seen.push(name),
                    backend::Update::TextDelta(_) => seen.push("text"),
                    _ => {}
                }
                Ok(())
            })
            .unwrap();
        seen
    };
    assert_eq!(path(&backend(&server.endpoint)), ["http", "text"]);
    assert_eq!(path(&backend(&server.endpoint)), ["socket", "text"]);
    server.finish();
}

#[test]
fn a_connection_lost_before_answering_is_replaced_and_the_request_sent_again() {
    let server = Server::start(|listener, stop| {
        let mut first = Peer::new(server::accept(&listener, &stop).unwrap());
        first.upgrade();
        first.message().unwrap();
        first.answer("One");
        let lost = first.message().unwrap();
        drop(first);
        let mut second = Peer::new(server::accept(&listener, &stop).unwrap());
        second.upgrade();
        let resent = second.message().unwrap();
        second.answer("Two");
        (lost, resent)
    });
    let backend = backend(&server.endpoint);
    assert_eq!(sample(&backend, "first").unwrap().text, "One");
    assert_eq!(sample(&backend, "second").unwrap().text, "Two");
    let (lost, resent) = server.finish();
    assert_eq!(lost, resent);
}

#[test]
fn a_connection_closed_while_idle_is_replaced_before_sending() {
    let server = Server::start(|listener, stop| {
        let mut first = Peer::new(server::accept(&listener, &stop).unwrap());
        first.upgrade();
        first.message().unwrap();
        first.answer("One");
        drop(first);
        let mut second = Peer::new(server::accept(&listener, &stop).unwrap());
        second.upgrade();
        second.message().unwrap();
        second.answer("Two");
    });
    let backend = backend(&server.endpoint);
    assert_eq!(sample(&backend, "first").unwrap().text, "One");
    // The close reaches the adapter between the two requests.
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(sample(&backend, "second").unwrap().text, "Two");
    server.finish();
}

#[test]
fn a_close_during_the_response_is_a_temporary_failure() {
    let server = Server::start(|listener, stop| {
        let mut peer = Peer::new(server::accept(&listener, &stop).unwrap());
        peer.upgrade();
        peer.message().unwrap();
        peer.send(&reply("Partial")[0]);
    });
    let backend = backend(&server.endpoint);
    let failure = sample(&backend, "prompt").err().unwrap();
    assert!(matches!(failure.kind, FailureKind::Temporary));
    assert!(failure.message.contains("closed during the response"));
    server.finish();
}

#[test]
fn error_events_keep_their_status_and_retry_delay_and_the_connection() {
    let server = Server::start(|listener, stop| {
        let mut peer = Peer::new(server::accept(&listener, &stop).unwrap());
        peer.upgrade();
        peer.message().unwrap();
        peer.send(
            r#"{"type":"error","status":429,"error":{"code":"rate_limit_exceeded","message":"Slow down"},"headers":{"retry-after":"7"}}"#,
        );
        peer.message().unwrap();
        peer.answer("Recovered");
    });
    let backend = backend(&server.endpoint);
    let failure = sample(&backend, "prompt").err().unwrap();
    assert!(matches!(failure.kind, FailureKind::Temporary));
    assert_eq!(failure.retry_after, Some(Duration::from_secs(7)));
    assert_eq!(sample(&backend, "again").unwrap().text, "Recovered");
    server.finish();
}

#[test]
fn an_expired_connection_is_replaced_within_the_same_request() {
    let server = Server::start(|listener, stop| {
        let mut first = Peer::new(server::accept(&listener, &stop).unwrap());
        first.upgrade();
        first.message().unwrap();
        first.send(
            r#"{"type":"error","status":400,"error":{"code":"websocket_connection_limit_reached","message":"Limit"}}"#,
        );
        let mut second = Peer::new(server::accept(&listener, &stop).unwrap());
        second.upgrade();
        second.message().unwrap();
        second.answer("Fresh");
    });
    let backend = backend(&server.endpoint);
    assert_eq!(sample(&backend, "prompt").unwrap().text, "Fresh");
    server.finish();
}

#[test]
fn cancellation_during_a_response_closes_the_connection() {
    let received = std::sync::Arc::new(std::sync::Barrier::new(2));
    let server = Server::start({
        let received = std::sync::Arc::clone(&received);
        move |listener, stop| {
            let mut peer = Peer::new(server::accept(&listener, &stop).unwrap());
            peer.upgrade();
            peer.message().unwrap();
            received.wait();
            // Stopping the adapter is what ends this read.
            peer.message().is_none()
        }
    });
    let mut backend = backend(&server.endpoint);
    let cancellation = Cancellation::default();
    backend.set_cancellation(cancellation.clone());
    let canceller = std::thread::spawn(move || {
        received.wait();
        cancellation.cancel();
    });
    let failure = sample(&backend, "prompt").err().unwrap();
    canceller.join().unwrap();
    assert_eq!(failure.message, "Operation cancelled");
    assert!(server.finish());
}
