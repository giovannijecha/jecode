use super::*;
use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    time::{Duration, Instant},
};

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
                    r#"{"issuer":"ISSUER","authorization_endpoint":"ISSUER/authorize","token_endpoint":"ISSUER/token","registration_endpoint":"ISSUER/register"}"#,
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

#[test]
fn metadata_discovery_falls_back_past_unusable_candidates() {
    let server = serve(4, |origin, path| match path {
        "/mcp" => (
            "401 Unauthorized",
            "{}".into(),
            "WWW-Authenticate: Bearer realm=mcp\r\n".into(),
        ),
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
                r#"{{"issuer":"{origin}","authorization_endpoint":"{origin}/authorize","token_endpoint":"{origin}/token","registration_endpoint":"{origin}/register"}}"#
            ),
            String::new(),
        ),
        _ => panic!("unexpected fixture path {path}"),
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

#[test]
fn browser_pkce_and_refresh_stay_on_discovered_issuer_and_resource() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let issuer = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
    let endpoint = format!("{issuer}/mcp?project_ref=fixture");
    let server_issuer = issuer.clone();
    let server_resource = format!("{issuer}/mcp");
    let expected_resource = server_resource.clone();
    let worker = std::thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(20);
        let mut auth_count = 0;
        let mut refresh_count = 0;
        for _ in 0..9 {
            let mut stream = loop {
                match listener.accept() {
                    Ok((stream, _)) => break stream,
                    Err(e)
                        if e.kind() == std::io::ErrorKind::WouldBlock
                            && Instant::now() < deadline =>
                    {
                        std::thread::sleep(Duration::from_millis(5))
                    }
                    Err(e) => panic!("OAuth fixture accept: {e}"),
                }
            };
            stream.set_nonblocking(false).unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut request = Vec::new();
            loop {
                let mut buf = [0; 4096];
                let n = stream.read(&mut buf).unwrap();
                if n == 0 {
                    break;
                }
                request.extend_from_slice(&buf[..n]);
                if let Some(pos) = request.windows(4).position(|p| p == b"\r\n\r\n") {
                    let head = std::str::from_utf8(&request[..pos]).unwrap();
                    let length = head
                        .lines()
                        .find_map(|line| {
                            line.to_ascii_lowercase()
                                .strip_prefix("content-length:")
                                .and_then(|s| s.trim().parse::<usize>().ok())
                        })
                        .unwrap_or(0);
                    if request.len() >= pos + 4 + length {
                        break;
                    }
                }
            }
            let text = std::str::from_utf8(&request).unwrap();
            let path = text.split_whitespace().nth(1).unwrap();
            let route = path.split('?').next().unwrap_or(path);
            if route == "/mcp" {
                assert!(text.contains("MCP-Protocol-Version: 2026-07-28"));
                assert!(text.contains("Mcp-Method: server/discover"));
            }
            let body = text
                .split_once("\r\n\r\n")
                .map(|(_, body)| body)
                .unwrap_or("");
            let (status, reply, extra) = match route {
                "/mcp" => (
                    "401 Unauthorized",
                    "{}".into(),
                    format!(
                        "WWW-Authenticate: Bearer resource_metadata=\"{server_issuer}/resource-info\"\r\n"
                    ),
                ),
                "/resource-info" => (
                    "200 OK",
                    format!(
                        r#"{{"resource":"{server_resource}","authorization_servers":["{server_issuer}"]}}"#
                    ),
                    String::new(),
                ),
                "/.well-known/oauth-authorization-server" => (
                    "200 OK",
                    format!(
                        r#"{{"issuer":"{server_issuer}","authorization_endpoint":"{server_issuer}/authorize?audience=fixture","token_endpoint":"{server_issuer}/token","registration_endpoint":"{server_issuer}/register","authorization_response_iss_parameter_supported":true}}"#
                    ),
                    String::new(),
                ),
                "/register" => (
                    "201 Created",
                    r#"{"client_id":"synthetic-client"}"#.into(),
                    String::new(),
                ),
                "/token" if body.contains("grant_type=authorization_code") => {
                    auth_count += 1;
                    assert!(body.contains("code_verifier="));
                    assert!(body.contains("resource="));
                    ("200 OK",r#"{"access_token":"access-one","refresh_token":"refresh-one","expires_in":120,"token_type":"Bearer"}"#.into(),String::new())
                }
                "/token" if body.contains("grant_type=refresh_token") => {
                    refresh_count += 1;
                    assert!(body.contains("refresh_token=refresh-one"));
                    assert!(body.contains("resource="));
                    ("200 OK",r#"{"access_token":"access-two","refresh_token":"refresh-two","expires_in":120,"token_type":"Bearer"}"#.into(),String::new())
                }
                _ => panic!("unexpected OAuth fixture request: {path}"),
            };
            let response = format!(
                "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n{extra}\r\n{reply}",
                reply.len()
            );
            stream.write_all(response.as_bytes()).unwrap();
        }
        assert_eq!(auth_count, 1);
        assert_eq!(refresh_count, 1);
    });
    let mut connection = Connection::draft("fixture", &endpoint);
    connection.method = AuthMethod::Browser;
    let expected_issuer = issuer.clone();
    let signed=sign_in(connection,&Cancellation::default(),&mut |update|{
        if let Update::Browser(url)=update {
            assert!(url.contains("audience=fixture&response_type=code"));
            let params=wire::query(url.split_once('?').unwrap().1).unwrap();
            let redirect=params.get("redirect_uri").unwrap().clone();
            let state=params.get("state").unwrap().clone();
            let target=redirect.strip_prefix("http://127.0.0.1:").unwrap();
            let port=target.split('/').next().unwrap().parse::<u16>().unwrap();
            let issuer=wire::encode(&expected_issuer);
            std::thread::spawn(move||{
                std::thread::sleep(Duration::from_millis(100));
                let mut stream=std::net::TcpStream::connect(("127.0.0.1",port)).unwrap();
                let request=format!("GET /callback?code=synthetic-code&state={state}&iss={issuer} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n");
                stream.write_all(request.as_bytes()).unwrap();
            });
        }
        Ok(())
    }).unwrap();
    assert_eq!(signed.oauth.as_ref().unwrap().access_token, "access-one");
    // The registered client belongs to this sign-in's redirect, not the user override.
    assert_eq!(signed.oauth.as_ref().unwrap().client_id, "synthetic-client");
    assert_eq!(signed.oauth_client_id, None);
    assert_eq!(
        signed.oauth.as_ref().unwrap().resource.as_deref(),
        Some(expected_resource.as_str())
    );
    let mut changed = signed.clone();
    changed.endpoint = format!("{issuer}/mcp?project_ref=other");
    assert!(super::super::config::validate(&changed).is_err());
    assert!(refresh(&changed, &Cancellation::default()).is_err());
    let renewed = refresh(&signed, &Cancellation::default()).unwrap();
    assert_eq!(renewed.oauth.as_ref().unwrap().access_token, "access-two");
    assert_eq!(
        renewed.oauth.as_ref().unwrap().refresh_token.as_deref(),
        Some("refresh-two")
    );
    worker.join().unwrap();
}
