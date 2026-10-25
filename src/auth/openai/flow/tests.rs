use super::*;
use std::io::{Read, Write};

fn send_callback(port: u16, target: &[u8]) -> String {
    let mut client = TcpStream::connect(("127.0.0.1", port)).unwrap();
    client
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    let mut request = b"GET ".to_vec();
    request.extend_from_slice(target);
    request.extend_from_slice(format!(" HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n\r\n").as_bytes());
    client.write_all(&request).unwrap();
    let mut response = String::new();
    client.read_to_string(&mut response).unwrap();
    response
}

#[test]
fn callback_accepts_fragmented_headers() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let receiver = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        handle_callback(&mut stream, port, "expected", &Cancellation::default())
    });
    let mut client = TcpStream::connect(("127.0.0.1", port)).unwrap();
    client
        .write_all(b"GET /auth/callback?code=secret&state=expected HTTP/1.1\r\nHo")
        .unwrap();
    std::thread::sleep(Duration::from_millis(20));
    client
        .write_all(format!("st: 127.0.0.1:{port}\r\n\r\n").as_bytes())
        .unwrap();
    assert_eq!(receiver.join().unwrap().unwrap().unwrap()["code"], "secret");
}

#[test]
fn invalid_callbacks_leave_listener_ready_for_valid_callback() {
    for invalid in [
        b"/auth/callback?code=secret&state=wrong".as_slice(),
        b"/auth/callback?code=secret&state=%XZ",
        b"/auth/callback?code=secret&state=expected&state=expected",
        b"/auth/callback?state=expected",
        b"/auth/callback?code=secret&state=expected\xff",
        b"/auth/callback?error=access_denied&state=wrong",
    ] {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let receiver = std::thread::spawn(move || {
            await_callback(&listener, "expected", &Cancellation::default())
        });
        let response = send_callback(port, invalid);
        assert!(response.starts_with("HTTP/1.1 400 Bad Request\r\n"));
        let success = send_callback(port, b"/auth/callback?code=valid&state=expected");
        assert!(success.starts_with("HTTP/1.1 200 OK\r\n"));
        assert_eq!(receiver.join().unwrap().unwrap()["code"], "valid");
    }
}

#[test]
fn matching_provider_error_ends_sign_in() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let receiver =
        std::thread::spawn(move || await_callback(&listener, "expected", &Cancellation::default()));
    let response = send_callback(port, b"/auth/callback?error=access_denied&state=expected");
    assert!(response.starts_with("HTTP/1.1 200 OK\r\n"));
    assert_eq!(
        receiver.join().unwrap().unwrap_err(),
        "OpenAI authorization was declined: access_denied"
    );
}

#[test]
fn refresh_token_is_retained_when_omitted_and_replaced_when_issued() {
    let omitted = Value::object([("access_token", Value::string("new-access"))]);
    assert_eq!(
        refresh_token(&omitted, "old-refresh").unwrap(),
        "old-refresh"
    );
    let rotated = Value::object([("refresh_token", Value::string("new-refresh"))]);
    assert_eq!(
        refresh_token(&rotated, "old-refresh").unwrap(),
        "new-refresh"
    );
}

#[test]
fn malformed_issued_refresh_token_is_rejected() {
    for invalid in [Value::Null, Value::number(1), Value::string("")] {
        let response = Value::object([("refresh_token", invalid)]);
        assert!(refresh_token(&response, "old-refresh").is_err());
    }
}

#[test]
fn browser_opener_rejects_non_https_and_malformed_links() {
    for url in [
        "http://example.test/usage",
        "https://",
        "https:///usage",
        "https://user@example.test/usage",
        "https://example.test/with space",
    ] {
        assert_eq!(open_browser(url).unwrap_err(), "Invalid HTTPS link");
    }
}

#[test]
fn unsupported_earliest_refresh_format_fails_closed() {
    let response = Value::object([("earliest_refresh_at", Value::string("tomorrow"))]);
    assert!(earliest_refresh(&response, 1).is_err());
    let response = Value::object([("earliest_refresh_at", Value::number(42))]);
    assert_eq!(earliest_refresh(&response, 1).unwrap(), 42);
}
