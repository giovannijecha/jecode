use super::*;
use crate::test_support::server::{self, Server};
use std::io::{BufRead, BufReader, Read, Write};
use std::time::Duration;

#[test]
fn input_error_preserves_cli_hint_order_and_updates_diagnostic() {
    let value = Value::object([(
        "error",
        Value::object([("message", Value::string("Key rejected"))]),
    )]);
    let failure = with_input_error(Failure::api(&value, 401), Some("input closed"));
    assert_eq!(
        failure.message,
        "OpenRouter HTTP 401: Key rejected. Check your key with jecode setup.; input closed"
    );
    assert_eq!(
        failure.diagnostic.as_deref(),
        Some("OpenRouter HTTP 401: Key rejected.; input closed")
    );
}

fn round_trip(body: &str) -> (Vec<String>, Vec<u8>) {
    let server = Server::start(move |listener, stop| {
        let mut stream = server::accept(&listener, &stop)?;
        stream
            .set_read_timeout(Some(Duration::from_secs(15)))
            .unwrap();
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let mut headers = Vec::new();
        let mut length = 0;
        loop {
            let mut line = String::new();
            assert_ne!(reader.read_line(&mut line).unwrap(), 0, "missing headers");
            if line == "\r\n" {
                break;
            }
            if let Some((name, value)) = line.split_once(':')
                && name.eq_ignore_ascii_case("content-length")
            {
                length = value.trim().parse::<usize>().unwrap();
            }
            headers.push(line.trim_end().to_owned());
        }
        let mut body = vec![0; length];
        reader.read_exact(&mut body).unwrap();
        let response = b"{\"ok\":true}";
        write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", response.len()).unwrap();
        stream.write_all(response).unwrap();
        Ok::<_, std::io::Error>((headers, body))
    });

    let api = Api::fixture(&format!("{}/chat/completions", server.endpoint));
    let mut observed = Vec::new();
    let response = api.raw("POST", "/chat/completions", Some(body), &mut |part| {
        observed.extend_from_slice(part);
        Ok(())
    });
    let received = server.finish();
    assert_eq!(response.unwrap(), "{\"ok\":true}");
    let (headers, received) = received.unwrap();
    assert_eq!(observed, b"{\"ok\":true}\n200");
    assert!(
        headers
            .iter()
            .any(|line| line == "Authorization: Bearer isolated-fixture-key")
    );
    assert!(
        headers
            .iter()
            .any(|line| line.eq_ignore_ascii_case("Content-Type: application/json"))
    );
    assert!(headers.iter().any(|line| line == "Accept: */*"));
    (headers, received)
}

#[test]
fn large_json_request_preserves_bytes_auth_and_response_stream() {
    // Literal @ must remain data, including at a common buffer boundary.
    let prefix = "x".repeat(256 * 1024 - "{\"text\":\"".len());
    let body = format!(
        "{{\"text\":\"{}{}\\\"\\\\é🦀\"}}",
        prefix,
        "@".repeat(11 * 1024 * 1024)
    );
    let (_, received) = round_trip(&body);
    assert_eq!(received, body.as_bytes());
}

#[test]
fn large_multimodal_request_preserves_image_data() {
    let image = format!("data:image/png;base64,{}", "A".repeat(11 * 1024 * 1024));
    let body = Value::object([(
        "messages",
        Value::Array(vec![Value::object([
            ("role", Value::string("user")),
            (
                "content",
                Value::Array(vec![
                    Value::object([
                        ("type", Value::string("text")),
                        ("text", Value::string("What is shown?")),
                    ]),
                    Value::object([
                        ("type", Value::string("image_url")),
                        ("image_url", Value::object([("url", Value::string(image))])),
                    ]),
                ]),
            ),
        ])]),
    )])
    .encode();
    assert!(body.len() > 10 * 1024 * 1024);
    let (_, received) = round_trip(&body);
    assert_eq!(received, body.as_bytes());
}

fn incomplete_output(stdout: &[u8], stderr: &[u8], exit: i32) -> process::Output {
    process::Output {
        stdout: stdout.to_vec(),
        stderr: stderr.to_vec(),
        exit_code: Some(exit),
        input_error: Some("Could not send process input: pipe closed".into()),
        timed_out: false,
        stdout_bytes: stdout.len(),
        stderr_bytes: stderr.len(),
        stdout_truncated: false,
        cancelled: false,
    }
}

#[test]
fn request_failures_retain_native_http_and_input_diagnostics() {
    let api = Api::fixture("http://127.0.0.1:1/chat/completions");
    let native = api
        .response(incomplete_output(b"", b"native diagnostic", 26), None)
        .unwrap_err();
    for detail in ["exit 26", "native diagnostic", "pipe closed"] {
        assert!(native.message.contains(detail));
    }
    let delay = Some(Duration::from_secs(2));
    let http = api
        .response(
            incomplete_output(
                b"HTTP/1.1 503 Unavailable\r\n\r\n{\"error\":{\"message\":\"overloaded\"}}\n503",
                b"",
                0,
            ),
            delay,
        )
        .unwrap_err();
    assert_eq!(http.kind, crate::backend::FailureKind::Temporary);
    assert_eq!(http.retry_after, delay);
    assert!(http.message.contains("HTTP 503"));
    assert!(http.message.contains("overloaded"));
    assert!(http.message.contains("pipe closed"));
    let invalid = api
        .response(incomplete_output(&[0xff], b"", 0), None)
        .unwrap_err();
    assert!(invalid.message.contains("invalid UTF-8"));
    assert!(invalid.message.contains("pipe closed"));
    let success = api
        .response(
            incomplete_output(b"HTTP/1.1 200 OK\r\n\r\n{}\n200", b"", 0),
            None,
        )
        .unwrap_err();
    assert!(success.message.contains("pipe closed"));
}

#[test]
fn old_curl_reports_the_required_version() {
    let api = Api::fixture("http://127.0.0.1:1/chat/completions");
    let mut output = incomplete_output(b"", b"curl: option --variable: is unknown", 2);
    output.input_error = None;
    let error = api.response(output, None).unwrap_err();
    assert!(error.message.contains("--variable"));
    assert!(error.message.contains("curl (exit 2)"));
    assert!(error.message.contains("version 8.3 or newer"));
    assert!(error.message.contains("JECODE_CURL"));
}
