use jecode::providers::openai_account::{
    ContentKind, Error, HttpResponseStream, Input, Progress, Request, encode_http,
};
use std::{
    io::{Read, Write},
    net::{Shutdown, TcpListener, TcpStream},
    ops::ControlFlow,
    time::Duration,
};

const SSE: &str = "data: {\"type\":\"response.output_text.delta\",\"delta\":\"Hello\"}\n\ndata: {\"type\":\"response.completed\",\"response\":{\"id\":\"r1\",\"status\":\"completed\",\"output\":[{\"type\":\"message\",\"content\":[{\"type\":\"output_text\",\"text\":\"Hello\"}]}]}}\n\n";

fn response(body: &str) -> Vec<u8> {
    let mut wire = b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream; charset=utf-8\r\nTransfer-Encoding: chunked\r\n\r\n".to_vec();
    for chunk in body.as_bytes().chunks(11) {
        wire.extend_from_slice(format!("{:x}\r\n", chunk.len()).as_bytes());
        wire.extend_from_slice(chunk);
        wire.extend_from_slice(b"\r\n");
    }
    wire.extend_from_slice(b"0\r\n\r\n");
    wire
}
fn request() -> Request {
    Request {
        model: "gpt-5.6-luna".into(),
        instructions: "Answer briefly.".into(),
        input: vec![Input::User("Say hello".into())],
        tools: vec![],
        effort: "low".into(),
    }
}

#[test]
fn complete_http_sse_pipeline_survives_every_chunk_boundary() {
    let wire = response(SSE);
    for split in 0..=wire.len() {
        let mut decoder = HttpResponseStream::default();
        let mut text = String::new();
        let mut callback = |event: Progress<'_>| {
            if let Progress::Text(delta) = event {
                text.push_str(delta);
            }
            ControlFlow::Continue(())
        };
        decoder.push(&wire[..split], &mut callback).unwrap();
        if !decoder.is_finished() {
            decoder.push(&wire[split..], &mut callback).unwrap();
        }
        assert!(decoder.is_finished());
        assert_eq!(decoder.finish().unwrap().text, "Hello");
        assert_eq!(text, "Hello");
    }
}

#[test]
fn account_stream_without_content_type_still_requires_valid_model_completion() {
    // The account backend can omit Content-Type while sending HTTP chunked SSE.
    let wire = response(SSE);
    let wire = String::from_utf8(wire)
        .unwrap()
        .replace("Content-Type: text/event-stream; charset=utf-8\r\n", "")
        .into_bytes();
    for split in 0..=wire.len() {
        let mut decoder = HttpResponseStream::default();
        let mut text = String::new();
        let mut progress = |event: Progress<'_>| {
            if let Progress::Text(delta) = event {
                text.push_str(delta);
            }
            ControlFlow::Continue(())
        };
        decoder.push(&wire[..split], &mut progress).unwrap();
        if !decoder.is_finished() {
            decoder.push(&wire[split..], &mut progress).unwrap();
        }
        assert_eq!(decoder.finish().unwrap().text, "Hello");
        assert_eq!(text, "Hello");
    }
    for body in [
        "",
        "<html>not a stream</html>",
        "{\"error\":\"synthetic-sensitive-body\"}",
        "data: {\"type\":\"response.created\"}\n\n",
        "data: [DONE]\n\n",
    ] {
        let wire = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        );
        let mut decoder = HttpResponseStream::default();
        let _ = decoder.push(wire.as_bytes(), |_| panic!("not model output"));
        assert!(
            decoder.finish().is_err(),
            "accepted missing terminal: {body}"
        );
    }
}

#[test]
fn error_status_redirect_and_non_stream_content_never_deliver_model_events() {
    for (head, expected) in [
        (
            "HTTP/1.1 401 Unauthorized\r\nContent-Type: application/json\r\n\r\n",
            Error::HttpStatus(401),
        ),
        (
            "HTTP/1.1 302 Found\r\nLocation: https://other.test/\r\n\r\n",
            Error::HttpStatus(302),
        ),
        (
            "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\n\r\n",
            Error::ContentType(ContentKind::Html),
        ),
    ] {
        let mut decoder = HttpResponseStream::default();
        assert_eq!(
            decoder.push(
                format!("{head}synthetic-sensitive-error").as_bytes(),
                |_| panic!("unexpected model data")
            ),
            Err(expected)
        );
        assert_eq!(decoder.finish(), Err(expected));
        assert!(!expected.to_string().contains("synthetic-sensitive-error"));
    }
}

#[test]
fn cancellation_and_premature_eof_do_not_return_success() {
    let mut decoder = HttpResponseStream::default();
    assert_eq!(
        decoder.push(&response(SSE), |_| ControlFlow::Break(())),
        Err(Error::Cancelled)
    );
    assert_eq!(decoder.finish(), Err(Error::Cancelled));
    let mut decoder = HttpResponseStream::default();
    let wire = response("data: {\"type\":\"response.output_text.delta\",\"delta\":\"Hello\"}\n\n");
    assert_eq!(
        decoder.push(&wire, |_| ControlFlow::Continue(())),
        Err(Error::MissingTerminal)
    );
    assert_eq!(decoder.finish(), Err(Error::MissingTerminal));
    let mut decoder = HttpResponseStream::default();
    decoder.push(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: 999\r\n\r\ndata: {", |_| ControlFlow::Continue(())).unwrap();
    assert_eq!(
        decoder.finish(),
        Err(Error::Http(jecode::http::Error::Truncated))
    );
    let mut decoder = HttpResponseStream::default();
    decoder.cancel();
    assert_eq!(decoder.finish(), Err(Error::Cancelled));
}

#[test]
fn non_stream_diagnostics_use_only_allowlisted_metadata() {
    for (field, kind, label) in [
        (
            "Content-Type: Application/JSON; charset=utf-8\r\n",
            ContentKind::Json,
            "JSON",
        ),
        ("Content-Type: text/html\r\n", ContentKind::Html, "HTML"),
        (
            "Content-Type: text/plain\r\n",
            ContentKind::Text,
            "plain text",
        ),
        (
            "Content-Type: synthetic-sensitive-header\r\n",
            ContentKind::Other,
            "unsupported Content-Type",
        ),
    ] {
        let wire = format!("HTTP/1.1 200 OK\r\n{field}\r\nsynthetic-sensitive-body");
        for split in 0..=wire.len() {
            let mut decoder = HttpResponseStream::default();
            let first = decoder.push(&wire.as_bytes()[..split], |_| panic!("not a model stream"));
            let result = if first.is_err() {
                first
            } else {
                decoder.push(&wire.as_bytes()[split..], |_| panic!("not a model stream"))
            };
            let expected = Error::ContentType(kind);
            assert_eq!(result, Err(expected));
            assert_eq!(decoder.finish(), Err(expected));
            assert!(expected.to_string().contains(label));
            assert!(!expected.to_string().contains("synthetic-sensitive"));
        }
    }
    let wire =
        format!("HTTP/1.1 200 OK\r\nContent-Type: TEXT/Event-Stream ; charset=utf-8\r\n\r\n{SSE}");
    let mut decoder = HttpResponseStream::default();
    decoder
        .push(wire.as_bytes(), |_| ControlFlow::Continue(()))
        .unwrap();
    assert_eq!(decoder.finish().unwrap().text, "Hello");
}

#[test]
fn loopback_exchange_integrates_request_http_body_and_model_stream() {
    // Plaintext exists only in this isolated fixture, never as a production account option.
    let expected = encode_http(&request(), "synthetic-token", "synthetic-account").unwrap();
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let mut client =
        TcpStream::connect_timeout(&listener.local_addr().unwrap(), Duration::from_secs(3))
            .unwrap();
    client
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    client
        .set_write_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    let sent = expected.clone();
    let server = std::thread::spawn(move || {
        let (mut peer, _) = listener.accept().unwrap();
        peer.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
        peer.set_write_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let mut received = Vec::new();
        Read::by_ref(&mut peer)
            .take(65536)
            .read_to_end(&mut received)
            .unwrap();
        assert_eq!(received, expected);
        peer.write_all(&response(SSE)).unwrap();
    });
    client.write_all(&sent).unwrap();
    client.shutdown(Shutdown::Write).unwrap();
    let mut decoder = HttpResponseStream::default();
    let mut buffer = [0; 13];
    while !decoder.is_finished() {
        let count = client.read(&mut buffer).unwrap();
        if count == 0 {
            break;
        }
        decoder
            .push(&buffer[..count], |_| ControlFlow::Continue(()))
            .unwrap();
    }
    drop(client);
    server.join().unwrap();
    assert_eq!(decoder.finish().unwrap().text, "Hello");
}

#[test]
fn account_request_pins_host_and_rejects_header_injection() {
    let text =
        String::from_utf8(encode_http(&request(), "synthetic-token", "account-test").unwrap())
            .unwrap();
    assert!(
        text.starts_with("POST /backend-api/codex/responses HTTP/1.1\r\nHost: chatgpt.com\r\n")
    );
    assert!(text.contains("\"model\":\"gpt-5.6-luna\""));
    for token in ["", "secret\r\nHost: other.test", "secret with spaces"] {
        assert_eq!(
            encode_http(&request(), token, "account-test"),
            Err(Error::InvalidRequest)
        );
    }
    assert!(encode_http(&request(), "synthetic-token", "account\r\nHost: other.test").is_err());
}
