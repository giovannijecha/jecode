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
    write!(stream, "HTTP/1.1 {status} OK\r\nContent-Length: {}\r\nContent-Type: application/json\r\n{headers}\r\n{body}", body.len()).unwrap();
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
                write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\n\r\n", event.len()).unwrap();
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
            r#"{"jsonrpc":"2.0","id":3,"error":{"code":-32602,"message":"untrusted-secret"}}"#,
        );
        requests
    });
    let mut client = Client::new(Connection::draft("GitHub", &server.endpoint));
    let error = client
        .call("create_issue", &Value::object([]), &Cancellation::default())
        .unwrap_err();
    assert_eq!(error.kind, ErrorKind::Remote);
    assert!(error.message.contains("-32602"));
    assert!(!error.message.contains("untrusted-secret"));
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
