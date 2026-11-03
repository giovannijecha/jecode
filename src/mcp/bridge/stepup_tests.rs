use super::*;
use crate::mcp::types::{AuthMethod, ClientAuthMethod, OAuthTokens};
use crate::test_support::Directory;
use crate::test_support::server::{Server, accept};
use std::io::{Read, Write};
use std::net::TcpStream;

fn read_request(stream: &mut TcpStream) -> String {
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let mut bytes = Vec::new();
    loop {
        let mut chunk = [0; 4096];
        let len = stream.read(&mut chunk).unwrap();
        assert!(len > 0);
        bytes.extend_from_slice(&chunk[..len]);
        if let Some(end) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
            let head = String::from_utf8_lossy(&bytes[..end]);
            let body_len = head
                .lines()
                .find_map(|line| {
                    line.to_ascii_lowercase()
                        .strip_prefix("content-length:")
                        .and_then(|length| length.trim().parse::<usize>().ok())
                })
                .unwrap_or(0);
            if bytes.len() >= end + 4 + body_len {
                return String::from_utf8(bytes).unwrap();
            }
        }
    }
}

fn reply(stream: &mut TcpStream, status: u16, extra: &str, body: &str) {
    write!(
        stream,
        "HTTP/1.1 {status} Result\r\nContent-Type: application/json\r\nContent-Length: {}\r\n{extra}\r\n{body}",
        body.len()
    )
    .unwrap();
}

fn challenged_call(challenge: &'static str) -> (Value, Option<String>, Vec<String>) {
    let server = Server::start(move |listener, stop| {
        let mut requests = Vec::new();
        for (status, extra, body) in [
            (
                200,
                "",
                r#"{"jsonrpc":"2.0","id":1,"result":{"supportedVersions":["2026-07-28"]}}"#,
            ),
            (
                200,
                "",
                r#"{"jsonrpc":"2.0","id":2,"result":{"tools":[{"name":"write","inputSchema":{"type":"object"}}]}}"#,
            ),
            (403, challenge, "{}"),
        ] {
            let mut stream = accept(&listener, &stop).unwrap();
            requests.push(read_request(&mut stream));
            reply(&mut stream, status, extra, body);
        }
        requests
    });
    let home = Directory::new();
    let store = Store::new(home.path().to_path_buf());
    let endpoint = server.endpoint.clone();
    let mut connection = Connection::draft("Fixture", &endpoint);
    connection.method = AuthMethod::Browser;
    connection.oauth = Some(OAuthTokens {
        endpoint: endpoint.clone(),
        access_token: "fixture-access".into(),
        refresh_token: None,
        expires_at: Some(4_000_000_000),
        token_endpoint: format!("{endpoint}/token"),
        client_id: "fixture-client".into(),
        client_auth_method: ClientAuthMethod::None,
        client_secret: None,
        client_secret_expires_at: None,
        issuer: endpoint.clone(),
        scope: Some("read".into()),
        resource: Some(endpoint),
        github: false,
    });
    store.save(&connection).unwrap();
    let result = execute_with(
        &Runtime::default(),
        Some(home.path()),
        &Value::object([
            ("action", Value::string("call")),
            ("connection", Value::string("Fixture")),
            ("tool", Value::string("write")),
        ]),
        &Cancellation::default(),
        &Redactor::empty(),
        None,
    );
    let pending = store.list().unwrap()[0].pending_scope.clone();
    (result, pending, server.finish())
}

#[test]
fn tool_specific_scope_challenge_is_saved_without_replaying_the_call() {
    let (result, pending, requests) = challenged_call(
        "WWW-Authenticate: Bearer error=\"insufficient_scope\", scope=\"write\"\r\n",
    );
    assert_eq!(result.get("sign_in_required"), Some(&Value::Bool(true)));
    assert_eq!(
        result.get("required_scope").and_then(Value::as_str),
        Some("write")
    );
    assert_eq!(pending.as_deref(), Some("write"));
    assert_eq!(requests.len(), 3);
    assert_eq!(
        requests
            .iter()
            .filter(|request| request.contains("\"method\":\"tools/call\""))
            .count(),
        1
    );
}

#[test]
fn bearer_403_without_scope_does_not_create_a_pending_scope() {
    let (result, pending, requests) =
        challenged_call("WWW-Authenticate: Bearer error=\"insufficient_scope\"\r\n");
    assert_eq!(result.get("sign_in_required"), Some(&Value::Bool(true)));
    assert!(result.get("required_scope").is_none());
    assert_eq!(pending, None);
    assert_eq!(requests.len(), 3);
}

fn scope_error() -> Error {
    let mut error = Error::new(ErrorKind::Authentication, "MCP server requires more access");
    error.status = Some(403);
    error.headers = vec![(
        "WWW-Authenticate".into(),
        "Bearer error=\"insufficient_scope\", scope=\"write\"".into(),
    )];
    error
}

#[test]
fn scope_storage_failure_keeps_the_challenge_and_explains_the_missing_record() {
    let home = Directory::new();
    let store = Store::new(home.path().to_path_buf());
    let mut connection = Connection::draft("Fixture", "https://fixture.test/mcp");
    connection.id = "mcp-fixture".into();
    connection.method = AuthMethod::Browser;
    std::fs::write(home.path().join("mcp.json"), "invalid JSON").unwrap();
    let mut error = scope_error();
    remember_scope(&store, &connection, &mut error);
    assert_eq!(error.kind, ErrorKind::Authentication);
    assert_eq!(error.status, Some(403));
    assert_eq!(
        crate::mcp::auth::scopes::required(&error).as_deref(),
        Some("write")
    );
    assert!(error.message.starts_with("MCP server requires more access"));
    assert!(
        error
            .message
            .contains("Could not save requested permissions")
    );
}

#[test]
fn a_removed_session_does_not_replace_the_original_challenge_with_a_store_error() {
    let home = Directory::new();
    let store = Store::new(home.path().to_path_buf());
    let mut connection = Connection::draft("Fixture", "https://fixture.test/mcp");
    connection.id = "mcp-removed".into();
    connection.method = AuthMethod::Browser;
    let mut error = scope_error();
    remember_scope(&store, &connection, &mut error);
    assert_eq!(error.message, "MCP server requires more access");
    assert_eq!(
        crate::mcp::auth::scopes::required(&error).as_deref(),
        Some("write")
    );
}

#[test]
fn pending_permissions_do_not_change_a_queued_calls_target() {
    let saved = Connection::draft("Fixture", "https://fixture.test/mcp?project=one");
    let mut current = saved.clone();
    current.pending_scope = Some("deploy".into());
    assert!(same_target(&saved, &current));
    current.endpoint = "https://fixture.test/mcp?project=two".into();
    assert!(!same_target(&saved, &current));
    current.endpoint.clone_from(&saved.endpoint);
    current.enabled = false;
    assert!(!same_target(&saved, &current));
}
