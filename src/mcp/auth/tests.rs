use super::*;
use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    time::{Duration, Instant},
};

mod metadata;

fn request(stream: &mut TcpStream) -> String {
    let mut bytes = Vec::new();
    loop {
        let mut buffer = [0; 4096];
        let size = stream.read(&mut buffer).unwrap();
        assert!(size > 0, "Incomplete HTTP fixture request");
        bytes.extend_from_slice(&buffer[..size]);
        if let Some(end) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
            let head = String::from_utf8_lossy(&bytes[..end]);
            let length = head
                .lines()
                .find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("Content-Length")
                        .then(|| value.trim().parse::<usize>().unwrap())
                })
                .unwrap_or(0);
            if bytes.len() >= end + 4 + length {
                return String::from_utf8(bytes).unwrap();
            }
        }
        assert!(bytes.len() <= 64 * 1024, "Oversize fixture request");
    }
}

fn fixture(protected: bool, public_metadata: bool) -> (String, std::thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let origin = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
    let endpoint = format!("{origin}/mcp");
    let copy = origin.clone();
    let worker = std::thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(10);
        let count = if protected { 3 } else { 4 };
        for _ in 0..count {
            let mut stream = loop {
                match listener.accept() {
                    Ok((stream, _)) => break stream,
                    Err(error)
                        if error.kind() == std::io::ErrorKind::WouldBlock
                            && Instant::now() < deadline =>
                    {
                        std::thread::sleep(Duration::from_millis(5))
                    }
                    Err(error) => panic!("fixture accept: {error}"),
                }
            };
            stream.set_nonblocking(false).unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let request = request(&mut stream);
            let path = request.split_whitespace().nth(1).unwrap();
            if path == "/mcp" {
                assert!(request.contains("MCP-Protocol-Version: 2026-07-28"));
                if request.contains("\"method\":\"server/discover\"") {
                    assert!(request.contains("Mcp-Method: server/discover"));
                } else {
                    assert!(request.contains("Mcp-Method: tools/list"));
                }
            }
            let (status, body, extra) = match path {
                "/mcp"
                    if protected
                        || (public_metadata && request.contains("\"method\":\"tools/list\"")) =>
                {
                    (
                        "401 Unauthorized",
                        "{}",
                        format!(
                            "WWW-Authenticate: Bearer resource_metadata=\"{copy}/.well-known/oauth-protected-resource/mcp\", scope=\"repo:read\"\r\n"
                        ),
                    )
                }
                "/mcp" => ("200 OK", "{}", String::new()),
                "/.well-known/oauth-protected-resource/mcp" if protected || public_metadata => (
                    "200 OK",
                    r#"{"resource":"RESOURCE","authorization_servers":["ISSUER"],"scopes_supported":["other"]}"#,
                    String::new(),
                ),
                "/.well-known/oauth-authorization-server" => (
                    "200 OK",
                    r#"{"issuer":"ISSUER","authorization_endpoint":"ISSUER/authorize","token_endpoint":"ISSUER/token","registration_endpoint":"ISSUER/register","code_challenge_methods_supported":["S256"]}"#,
                    String::new(),
                ),
                "/.well-known/oauth-protected-resource/mcp"
                | "/.well-known/oauth-protected-resource" => ("404 Not Found", "{}", String::new()),
                _ => panic!("unexpected fixture path {path}"),
            };
            let body = body.replace("RESOURCE", &endpoint).replace("ISSUER", &copy);
            let response = format!(
                "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n{extra}\r\n{body}",
                body.len()
            );
            stream.write_all(response.as_bytes()).unwrap();
        }
    });
    (format!("{origin}/mcp"), worker)
}

#[test]
fn checked_public_endpoint_offers_only_none() {
    let (endpoint, worker) = fixture(false, false);
    let connection = Connection::draft("fixture", &endpoint);
    assert_eq!(
        methods(&connection, &Cancellation::default()).unwrap(),
        vec![AuthMethod::None]
    );
    worker.join().unwrap();
}

#[test]
fn auth_discovery_finishes_when_the_server_keeps_its_sse_response_open() {
    use crate::test_support::server::{Server, accept};
    let server = Server::start(|listener, stop| {
        let mut discovery = accept(&listener, &stop).unwrap();
        discovery
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        request(&mut discovery);
        write!(discovery, "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\n\r\ndata: {{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{{\"supportedVersions\":[\"2026-07-28\"]}}}}\n\n").unwrap();
        for index in 0..3 {
            let mut stream = accept(&listener, &stop).unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let text = request(&mut stream);
            let status = if index == 0 {
                assert!(text.contains("Mcp-Method: tools/list"));
                "200 OK"
            } else {
                "404 Not Found"
            };
            write!(stream, "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{{}}").unwrap();
        }
        drop(discovery);
    });
    let mut connection = Connection::draft("streaming", &format!("{}/mcp", server.endpoint));
    connection.timeout_secs = 2;
    assert_eq!(
        methods(&connection, &Cancellation::default()).unwrap(),
        vec![AuthMethod::None]
    );
    server.finish();
}

#[test]
fn checked_protected_endpoint_offers_token_and_discovered_browser() {
    let (endpoint, worker) = fixture(true, false);
    let connection = Connection::draft("fixture", &endpoint);
    assert_eq!(
        methods(&connection, &Cancellation::default()).unwrap(),
        vec![AuthMethod::Token, AuthMethod::Browser]
    );
    worker.join().unwrap();
}

#[test]
fn public_discovery_with_protected_catalog_requires_token_or_browser() {
    let (endpoint, worker) = fixture(false, true);
    let connection = Connection::draft("fixture", &endpoint);
    assert_eq!(
        methods(&connection, &Cancellation::default()).unwrap(),
        vec![AuthMethod::Token, AuthMethod::Browser]
    );
    worker.join().unwrap();
}

type Route = fn(&str, &str) -> (&'static str, String, String);

fn serve(count: usize, route: Route) -> crate::test_support::server::Server<()> {
    use crate::test_support::server::{Server, accept};
    Server::start(move |listener, stop| {
        let origin = format!("http://{}", listener.local_addr().unwrap());
        for _ in 0..count {
            let mut stream = accept(&listener, &stop).unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let text = request(&mut stream);
            let (status, body, extra) = route(&origin, text.split_whitespace().nth(1).unwrap());
            write!(stream, "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n{extra}\r\n{body}", body.len()).unwrap();
        }
    })
}

fn checked(server: &crate::test_support::server::Server<()>) -> Vec<AuthMethod> {
    let mut connection = Connection::draft("fixture", &format!("{}/mcp", server.endpoint));
    connection.timeout_secs = 2;
    methods(&connection, &Cancellation::default()).unwrap()
}

#[test]
fn unusable_metadata_on_a_public_server_still_offers_none() {
    let server = serve(4, |_, path| match path {
        "/mcp" => ("200 OK", "{}".into(), String::new()),
        "/.well-known/oauth-protected-resource/mcp" => {
            ("200 OK", "<html>login</html>".into(), String::new())
        }
        "/.well-known/oauth-protected-resource" => {
            ("500 Internal Server Error", "{}".into(), String::new())
        }
        _ => panic!("unexpected fixture path {path}"),
    });
    assert_eq!(checked(&server), vec![AuthMethod::None]);
    server.finish();
}

/// Protected-resource and server metadata behind an unusable first candidate.
fn discoverable(origin: &str, path: &str) -> (&'static str, String, String) {
    match path {
        "/.well-known/oauth-protected-resource/mcp" => {
            ("200 OK", "<html>login</html>".into(), String::new())
        }
        "/.well-known/oauth-protected-resource" => (
            "200 OK",
            format!(r#"{{"resource":"{origin}/mcp","authorization_servers":["{origin}"]}}"#),
            String::new(),
        ),
        "/.well-known/oauth-authorization-server" => (
            "200 OK",
            format!(
                r#"{{"issuer":"{origin}","authorization_endpoint":"{origin}/authorize","token_endpoint":"{origin}/token","registration_endpoint":"{origin}/register","code_challenge_methods_supported":["S256"]}}"#
            ),
            String::new(),
        ),
        _ => panic!("unexpected fixture path {path}"),
    }
}

#[test]
fn metadata_discovery_falls_back_past_unusable_candidates() {
    let server = serve(4, |origin, path| match path {
        "/mcp" => (
            "401 Unauthorized",
            "{}".into(),
            "WWW-Authenticate: Bearer realm=mcp\r\n".into(),
        ),
        _ => discoverable(origin, path),
    });
    assert_eq!(
        checked(&server),
        vec![AuthMethod::Token, AuthMethod::Browser]
    );
    server.finish();
}

#[test]
fn unsupported_client_auth_metadata_does_not_offer_browser_sign_in() {
    let server = serve(4, |origin, path| match path {
        "/mcp" => (
            "401 Unauthorized",
            "{}".into(),
            "WWW-Authenticate: Bearer realm=mcp\r\n".into(),
        ),
        "/.well-known/oauth-authorization-server" => (
            "200 OK",
            format!(
                r#"{{"issuer":"{origin}","authorization_endpoint":"{origin}/authorize","token_endpoint":"{origin}/token","registration_endpoint":"{origin}/register","code_challenge_methods_supported":["S256"],"token_endpoint_auth_methods_supported":["private_key_jwt"]}}"#
            ),
            String::new(),
        ),
        _ => discoverable(origin, path),
    });
    assert_eq!(checked(&server), vec![AuthMethod::Token]);
    server.finish();
}

#[test]
fn scope_challenge_is_checked_like_an_authentication_challenge() {
    let server = serve(4, |origin, path| match path {
        "/mcp" => (
            "403 Forbidden",
            "{}".into(),
            "WWW-Authenticate: Bearer error=\"insufficient_scope\", scope=\"repo\"\r\n".into(),
        ),
        _ => discoverable(origin, path),
    });
    assert_eq!(
        checked(&server),
        vec![AuthMethod::Token, AuthMethod::Browser]
    );
    server.finish();
}

#[test]
fn token_lifetimes_and_in_body_errors_are_understood() {
    let context = TokenContext {
        endpoint: "https://issuer.test/token",
        bound_endpoint: "https://example.test/mcp",
        client: "client",
        client_auth_method: ClientAuthMethod::None,
        client_secret: None,
        client_secret_expires_at: None,
        issuer: "https://issuer.test",
        resource: None,
        github: false,
        now: 10,
    };
    for lifetime in [Value::number("120.0"), Value::string("120")] {
        let value = Value::object([
            ("access_token", Value::string("synthetic")),
            ("expires_in", lifetime),
        ]);
        assert_eq!(
            token_record(&value, context, None).unwrap().expires_at,
            Some(130)
        );
    }
    let revoked = Value::object([("error", Value::string("bad_refresh_token"))]);
    let error = token_record(&revoked, context, None).map(drop).unwrap_err();
    assert_eq!(error.kind, super::super::types::ErrorKind::Authentication);
}

#[test]
fn token_record_rejects_expired_or_wrong_type() {
    let context = TokenContext {
        endpoint: "https://issuer.test/token",
        bound_endpoint: "https://example.test/mcp",
        client: "client",
        client_auth_method: ClientAuthMethod::None,
        client_secret: None,
        client_secret_expires_at: None,
        issuer: "https://issuer.test",
        resource: None,
        github: false,
        now: 10,
    };
    let expired = Value::object([
        ("access_token", Value::string("synthetic")),
        ("expires_in", Value::number(0)),
    ]);
    assert!(token_record(&expired, context, None).is_err());
    let wrong = Value::object([
        ("access_token", Value::string("synthetic")),
        ("token_type", Value::string("mac")),
    ]);
    assert!(token_record(&wrong, context, None).is_err());
}

#[test]
fn discovered_oauth_endpoint_may_use_a_different_secure_origin() {
    assert!(
        discovery::trusted_endpoint(
            "https://login.example.test/token",
            "https://accounts.example.test"
        )
        .is_ok()
    );
    assert!(
        discovery::trusted_endpoint(
            "http://127.0.0.1:8899/token",
            "https://accounts.example.test"
        )
        .is_err()
    );
}

mod client_auth;
