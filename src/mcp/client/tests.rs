use super::*;
use crate::test_support::server::{Server, accept};
use std::io::{Read, Write};
use std::net::TcpStream;

fn read_request(stream: &mut TcpStream) -> String {
    stream
        .set_read_timeout(Some(std::time::Duration::from_secs(5)))
        .unwrap();
    let mut bytes = Vec::new();
    let mut buffer = [0; 4096];
    loop {
        let count = stream.read(&mut buffer).unwrap();
        assert!(count > 0);
        bytes.extend_from_slice(&buffer[..count]);
        let Some(end) = bytes.windows(4).position(|part| part == b"\r\n\r\n") else {
            continue;
        };
        let head = String::from_utf8_lossy(&bytes[..end]);
        let length = head
            .lines()
            .find_map(|line| {
                line.to_ascii_lowercase()
                    .strip_prefix("content-length:")
                    .and_then(|value| value.trim().parse::<usize>().ok())
            })
            .unwrap_or(0);
        if bytes.len() >= end + 4 + length {
            return String::from_utf8(bytes).unwrap();
        }
    }
}

fn reply(stream: &mut TcpStream, status: u16, headers: &str, body: &str) {
    let kind = if headers.contains("Content-Type") {
        ""
    } else {
        "Content-Type: application/json\r\n"
    };
    write!(
        stream,
        "HTTP/1.1 {status} OK\r\nConnection: close\r\nContent-Length: {}\r\n{kind}{headers}\r\n{body}",
        body.len()
    )
    .unwrap();
}

type Script = Vec<(u16, &'static str, String)>;

/// Answers one connection per scripted reply and returns the requests.
fn scripted(replies: Script) -> Server<Vec<String>> {
    Server::start(move |listener, stop| {
        replies
            .into_iter()
            .map(|(status, headers, body)| {
                let mut stream = accept(&listener, &stop).unwrap();
                let request = read_request(&mut stream);
                reply(&mut stream, status, headers, &body);
                request
            })
            .collect()
    })
}

/// A Legacy `initialize` answer with request `id`, then the initialized ack.
fn initialized(id: usize, headers: &'static str, version: &str) -> Script {
    let result =
        format!(r#"{{"protocolVersion":"{version}","serverInfo":{{"name":"old","version":"1"}}}}"#);
    vec![
        (200, headers, fixture_response(id, &result)),
        (202, "", String::new()),
    ]
}

fn fixture_response(id: usize, result: &str) -> String {
    format!(r#"{{"jsonrpc":"2.0","id":{id},"result":{result}}}"#)
}

#[test]
fn modern_discovery_pagination_and_single_tool_dispatch() {
    let server = Server::start(|listener, stop| {
        let mut requests = Vec::new();
        for (id, result) in [
            (
                1,
                r#"{"supportedVersions":["2026-07-28"],"_meta":{"io.modelcontextprotocol/serverInfo":{"name":"demo","version":"1"}}}"#,
            ),
            (
                2,
                r#"{"tools":[{"name":"one","inputSchema":{"type":"object","x-custom":3,"properties":{"region":{"type":"string","x-mcp-header":"Region"}}}}],"nextCursor":"more"}"#,
            ),
            (
                3,
                r#"{"tools":[{"name":"two","inputSchema":{"type":"object"}}]}"#,
            ),
            (
                4,
                r#"{"content":[{"type":"text","text":"done"}],"isError":true}"#,
            ),
        ] {
            let mut stream = accept(&listener, &stop).unwrap();
            requests.push(read_request(&mut stream));
            if id == 4 {
                let event = format!("data: {}\n\n", fixture_response(id, result));
                write!(stream, "HTTP/1.1 200 OK\r\nConnection: close\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\n\r\n", event.len()).unwrap();
                for part in event.as_bytes().chunks(3) {
                    stream.write_all(part).unwrap();
                }
            } else {
                reply(&mut stream, 200, "", &fixture_response(id, result));
            }
        }
        requests
    });
    let mut client = Client::new(Connection::draft("demo", &server.endpoint));
    let cancel = Cancellation::default();
    let catalog = client.catalog(&cancel).unwrap();
    assert_eq!(catalog.server.name, "demo");
    assert_eq!(catalog.tools.len(), 2);
    assert_eq!(
        catalog.tools[0].input_schema.get("x-custom"),
        Some(&Value::number(3))
    );
    let result = client
        .call(
            "one",
            &Value::object([("region", Value::string(" padded "))]),
            &cancel,
        )
        .unwrap();
    assert_eq!(result.get("isError"), Some(&Value::Bool(true)));
    let requests = server.finish();
    assert!(requests[0].contains("Mcp-Method: server/discover"));
    assert!(requests[0].contains("io.modelcontextprotocol/clientCapabilities"));
    assert!(requests[2].contains("\"cursor\":\"more\""));
    assert!(requests[3].contains("Mcp-Name: one"));
    assert!(requests[3].contains("Mcp-Param-Region: =?base64?IHBhZGRlZCA=?="));
}

#[test]
fn legacy_fallback_preserves_session_and_does_not_retry_tool_call() {
    let server = Server::start(|listener, stop| {
        let mut requests = Vec::new();
        for (status, headers, body) in [
            (400, "", "unsupported".to_string()),
            (
                200,
                "Mcp-Session-Id: fixture\r\n",
                fixture_response(
                    2,
                    r#"{"protocolVersion":"2025-06-18","serverInfo":{"name":"old","version":"1"}}"#,
                ),
            ),
            (202, "", String::new()),
            (500, "", "failure".to_string()),
        ] {
            let mut stream = accept(&listener, &stop).unwrap();
            requests.push(read_request(&mut stream));
            reply(&mut stream, status, headers, &body);
        }
        requests
    });
    let mut client = Client::new(Connection::draft("old", &server.endpoint));
    let error = client
        .call("danger", &Value::object([]), &Cancellation::default())
        .unwrap_err();
    assert_eq!(error.kind, ErrorKind::UnknownOutcome);
    let requests = server.finish();
    assert_eq!(requests.len(), 4);
    assert!(requests[1].contains("\"method\":\"initialize\""));
    assert!(requests[2].contains("Mcp-Session-Id: fixture"));
    assert!(requests[3].contains("MCP-Protocol-Version: 2025-06-18"));
}

#[test]
fn modern_version_error_does_not_start_legacy_handshake() {
    let server = Server::start(|listener, stop| {
        let mut stream = accept(&listener, &stop).unwrap();
        let request = read_request(&mut stream);
        reply(
            &mut stream,
            400,
            "",
            r#"{"jsonrpc":"2.0","id":1,"error":{"code":-32022,"message":"version","data":{"supported":["other"]}}}"#,
        );
        request
    });
    let mut client = Client::new(Connection::draft("modern", &server.endpoint));
    let error = client.catalog(&Cancellation::default()).unwrap_err();
    assert_eq!(error.kind, ErrorKind::Unsupported);
    assert!(server.finish().contains("Mcp-Method: server/discover"));
}

#[test]
fn modern_version_error_can_negotiate_offered_legacy_version() {
    let server = Server::start(|listener, stop| {
        let mut methods = Vec::new();
        for (status, body) in [
            (400, r#"{"jsonrpc":"2.0","id":1,"error":{"code":-32022,"message":"version","data":{"supported":["2025-11-25"]}}}"#.to_string()),
            (200, fixture_response(2, r#"{"protocolVersion":"2025-11-25","serverInfo":{"name":"legacy","version":"1"}}"#)),
            (202, String::new()),
            (200, fixture_response(3, r#"{"tools":[]}"#)),
        ] {
            let mut stream = accept(&listener, &stop).unwrap();
            methods.push(read_request(&mut stream));
            reply(&mut stream, status, "", &body);
        }
        methods
    });
    let mut client = Client::new(Connection::draft("both", &server.endpoint));
    assert_eq!(
        client
            .catalog(&Cancellation::default())
            .unwrap()
            .server
            .name,
        "legacy"
    );
    let requests = server.finish();
    assert_eq!(requests.len(), 4);
    assert!(requests[1].contains("\"method\":\"initialize\""));
}

#[test]
fn predispatch_cancellation_is_not_an_unknown_outcome() {
    let mut client = Client::new(Connection::draft("stopped", "http://127.0.0.1:1/mcp"));
    let cancel = Cancellation::default();
    cancel.cancel();
    assert_eq!(
        client
            .call("write", &Value::object([]), &cancel)
            .unwrap_err()
            .kind,
        ErrorKind::Cancelled
    );
}

#[test]
fn input_required_is_incomplete_and_never_retried_automatically() {
    let server = Server::start(|listener, stop| {
        let mut requests = Vec::new();
        for (id, result) in [
            (1, r#"{"supportedVersions":["2026-07-28"]}"#),
            (
                2,
                r#"{"tools":[{"name":"interactive","inputSchema":{"type":"object"}}]}"#,
            ),
            (
                3,
                r#"{"resultType":"input_required","inputRequests":{"confirm":{"method":"elicitation/create","params":{}}}}"#,
            ),
        ] {
            let mut stream = accept(&listener, &stop).unwrap();
            requests.push(read_request(&mut stream));
            reply(&mut stream, 200, "", &fixture_response(id, result));
        }
        requests
    });
    let mut client = Client::new(Connection::draft("interactive", &server.endpoint));
    let error = client
        .call("interactive", &Value::object([]), &Cancellation::default())
        .unwrap_err();
    assert_eq!(error.kind, ErrorKind::Unsupported);
    assert!(error.message.contains("additional input"));
    assert_eq!(server.finish().len(), 3);
}

#[test]
fn confirmed_rpc_rejection_is_failed_rather_than_an_unknown_dispatch() {
    let server = Server::start(|listener, stop| {
        let mut requests = Vec::new();
        for (id, result) in [
            (1, r#"{"supportedVersions":["2026-07-28"]}"#),
            (
                2,
                r#"{"tools":[{"name":"create_issue","inputSchema":{"type":"object"}}]}"#,
            ),
        ] {
            let mut stream = accept(&listener, &stop).unwrap();
            requests.push(read_request(&mut stream));
            reply(&mut stream, 200, "", &fixture_response(id, result));
        }
        let mut stream = accept(&listener, &stop).unwrap();
        requests.push(read_request(&mut stream));
        reply(
            &mut stream,
            200,
            "",
            r#"{"jsonrpc":"2.0","id":3,"error":{"code":-32602,"message":"title is required"}}"#,
        );
        requests
    });
    let mut client = Client::new(Connection::draft("GitHub", &server.endpoint));
    let error = client
        .call("create_issue", &Value::object([]), &Cancellation::default())
        .unwrap_err();
    assert_eq!(error.kind, ErrorKind::Remote);
    assert!(error.message.contains("-32602: title is required"));
    assert_eq!(server.finish().len(), 3);
}

#[test]
fn sse_response_completes_while_server_keeps_stream_open() {
    let server = Server::start(|listener, stop| {
        let mut discover = accept(&listener, &stop).unwrap();
        read_request(&mut discover);
        reply(
            &mut discover,
            200,
            "",
            &fixture_response(1, r#"{"supportedVersions":["2026-07-28"]}"#),
        );
        drop(discover);
        let mut list = accept(&listener, &stop).unwrap();
        read_request(&mut list);
        write!(
            list,
            "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\n\r\n"
        )
        .unwrap();
        for fragment in [
            "data: {\"jsonrpc\":\"2.0\",\"id\":2,",
            "\"result\":{\"tools\":[]}}\n\n",
        ] {
            list.write_all(fragment.as_bytes()).unwrap();
            list.flush().unwrap();
        }
        let _ = stop.wait(std::time::Duration::from_secs(5));
    });
    let started = std::time::Instant::now();
    let mut client = Client::new(Connection::draft("persistent", &server.endpoint));
    assert!(
        client
            .catalog(&Cancellation::default())
            .unwrap()
            .tools
            .is_empty()
    );
    assert!(started.elapsed() < std::time::Duration::from_secs(3));
    server.finish();
}

#[test]
fn any_discovery_rejection_except_authentication_falls_back_to_initialize() {
    for (status, body) in [
        (405, String::new()),
        (500, "failure".into()),
        (
            400,
            r#"{"jsonrpc":"2.0","id":1,"error":{"code":-32000,"message":"No session"}}"#.into(),
        ),
        (
            200,
            r#"{"jsonrpc":"2.0","id":1,"error":{"code":-32602,"message":"Bad params"}}"#.into(),
        ),
    ] {
        let mut replies = vec![(status, "", body)];
        replies.extend(initialized(2, "", "2025-11-25"));
        replies.push((200, "", fixture_response(3, r#"{"tools":[]}"#)));
        let server = scripted(replies);
        let mut client = Client::new(Connection::draft("old", &server.endpoint));
        let catalog = client.catalog(&Cancellation::default());
        assert_eq!(catalog.unwrap().server.name, "old", "{status}");
        assert_eq!(server.finish().len(), 4);
    }
}

#[test]
fn scope_challenge_requires_sign_in_instead_of_fallback() {
    let server = scripted(vec![(
        403,
        "WWW-Authenticate: Bearer error=\"insufficient_scope\", scope=\"repo\"\r\n",
        String::new(),
    )]);
    let mut client = Client::new(Connection::draft("scoped", &server.endpoint));
    let error = client.catalog(&Cancellation::default()).unwrap_err();
    assert_eq!(error.kind, ErrorKind::Authentication);
    assert_eq!(error.status, Some(403));
    assert_eq!(error.headers.len(), 1);
    assert_eq!(server.finish().len(), 1);
}

#[test]
fn expired_legacy_session_is_renewed_once_for_catalog_and_call() {
    let mut replies = vec![(400, "", String::new())];
    replies.extend(initialized(2, "Mcp-Session-Id: first\r\n", "2025-11-25"));
    replies.push((404, "", String::new()));
    replies.extend(initialized(4, "Mcp-Session-Id: second\r\n", "2025-11-25"));
    replies.push((200, "", fixture_response(5, r#"{"tools":[]}"#)));
    replies.push((404, "", String::new()));
    replies.extend(initialized(7, "Mcp-Session-Id: third\r\n", "2025-11-25"));
    replies.push((200, "", fixture_response(8, r#"{"content":[]}"#)));
    let server = scripted(replies);
    let mut client = Client::new(Connection::draft("old", &server.endpoint));
    let cancel = Cancellation::default();
    assert!(client.catalog(&cancel).unwrap().tools.is_empty());
    assert!(client.call("write", &Value::object([]), &cancel).is_ok());
    let requests = server.finish();
    assert_eq!(requests.len(), 11);
    assert!(requests[3].contains("Mcp-Session-Id: first"));
    assert!(requests[4].contains("\"method\":\"initialize\""));
    assert!(requests[6].contains("Mcp-Session-Id: second"));
    assert!(requests[10].contains("Mcp-Session-Id: third"));
    assert!(requests[10].contains("\"method\":\"tools/call\""));
}

#[test]
fn definite_http_rejections_fail_while_server_errors_stay_unknown() {
    let mut replies = vec![(400, "", String::new())];
    replies.extend(initialized(2, "", "2025-11-25"));
    replies.push((429, "", String::new()));
    replies.push((
        403,
        "WWW-Authenticate: Bearer error=\"insufficient_scope\"\r\n",
        String::new(),
    ));
    replies.push((503, "", String::new()));
    let server = scripted(replies);
    let mut client = Client::new(Connection::draft("old", &server.endpoint));
    let cancel = Cancellation::default();
    let mut call = || {
        client
            .call("write", &Value::object([]), &cancel)
            .unwrap_err()
    };
    let rejected = call();
    assert_eq!(
        (rejected.kind, rejected.status),
        (ErrorKind::Remote, Some(429))
    );
    let scope = call();
    assert_eq!(scope.kind, ErrorKind::Authentication);
    assert_eq!(scope.headers.len(), 1);
    assert_eq!(call().kind, ErrorKind::UnknownOutcome);
    assert_eq!(server.finish().len(), 6);
}

#[test]
fn legacy_catalog_keeps_valid_tools_through_sse_noise() {
    let tools = r#"{"tools":[{"name":"kept","inputSchema":{"type":"object","properties":{"r":{"type":"object","x-mcp-header":"R"}}}},{"name":""},{"name":"bad","inputSchema":3}],"nextCursor":""}"#;
    let event = format!(
        "id: a\ndata: \n\ndata: {{\"jsonrpc\":\"2.0\",\"id\":3,\"method\":\"ping\"}}\n\ndata: {}\n\n",
        fixture_response(3, tools)
    );
    let mut replies = vec![(400, "", String::new())];
    replies.extend(initialized(2, "", "2025-03-26"));
    replies.push((200, "Content-Type: text/event-stream\r\n", event));
    let server = scripted(replies);
    let mut client = Client::new(Connection::draft("old", &server.endpoint));
    let catalog = client.catalog(&Cancellation::default()).unwrap();
    assert_eq!(catalog.tools.len(), 1);
    assert_eq!(catalog.tools[0].name, "kept");
    assert!(server.finish()[3].contains("MCP-Protocol-Version: 2025-03-26"));
}

#[path = "paging_tests.rs"]
mod paging;
