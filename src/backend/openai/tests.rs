use super::*;
use crate::test_support::server::{self, Server};
use std::io::{BufRead, BufReader, Read, Write};
use std::time::Duration;

type Fixture = Server<Result<(Vec<String>, String), std::io::Error>>;

fn serve(response_status: u16, response_body: impl Into<String>) -> (Fixture, String) {
    serve_with_headers(response_status, response_body, "")
}

fn serve_with_headers(
    response_status: u16,
    response_body: impl Into<String>,
    extra_headers: &'static str,
) -> (Fixture, String) {
    let response_body = response_body.into();
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
            if reader.read_line(&mut line)? == 0 {
                break;
            }
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
        reader.read_exact(&mut body)?;
        write!(
            stream,
            "HTTP/1.1 {response_status} Test\r\n{extra_headers}Content-Type: application/json\r\nx-request-id: req_fixture\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            response_body.len()
        )?;
        stream.write_all(response_body.as_bytes())?;
        Ok((headers, String::from_utf8(body).unwrap()))
    });
    let endpoint = server.endpoint.clone();
    (server, endpoint)
}

#[test]
fn large_stream_reaches_completion_beyond_the_process_capture_limit() {
    let text = "x".repeat(96 * 1024);
    let delta = Value::object([
        ("type", Value::string("response.output_text.delta")),
        ("output_index", Value::number(0)),
        ("delta", Value::string(&text)),
    ]);
    let terminal = Value::object([
        ("type", Value::string("response.completed")),
        (
            "response",
            Value::object([
                ("status", Value::string("completed")),
                (
                    "output",
                    Value::Array(vec![Value::object([
                        ("type", Value::string("message")),
                        ("role", Value::string("assistant")),
                        (
                            "content",
                            Value::Array(vec![Value::object([
                                ("type", Value::string("output_text")),
                                ("text", Value::string(&text)),
                            ])]),
                        ),
                    ])]),
                ),
            ]),
        ),
    ]);
    let (server, endpoint) = serve(
        200,
        format!(
            "data: {}\n\ndata: {}\n\n",
            delta.encode(),
            terminal.encode()
        ),
    );
    let api = Api::fixture(Session::fixture("fixture-access-token"), &endpoint);
    let backend = OpenAi::with_api(api, "model-a".into()).unwrap();
    let mut observed = String::new();
    let completion = Backend::sample(&backend, &[], false, None, None, &mut |update| {
        if let backend::Update::TextDelta(delta) = update {
            observed.push_str(&delta);
        }
        Ok(())
    })
    .unwrap();
    assert_eq!(observed.len(), text.len());
    assert!(observed == text);
    assert!(completion.text == text);
    server.finish().unwrap();
}

#[test]
fn retry_after_survives_http_and_stream_failures() {
    for (status, body) in [
        (
            429,
            r#"{"error":{"code":"rate_limit_exceeded","message":"Wait"}}"#,
        ),
        (
            200,
            "data: {\"type\":\"response.failed\",\"response\":{\"error\":{\"code\":\"rate_limit_exceeded\",\"message\":\"Wait\"}}}\n\n",
        ),
    ] {
        let (server, endpoint) = serve_with_headers(status, body, "rEtRy-AfTeR: 90\r\n");
        let api = Api::fixture(Session::fixture("fixture-access-token"), &endpoint);
        let backend = OpenAi::with_api(api, "model-a".into()).unwrap();
        let failure = Backend::sample(&backend, &[], false, None, None, &mut |_| Ok(()))
            .err()
            .unwrap();
        assert_eq!(failure.kind, backend::FailureKind::Temporary);
        assert_eq!(failure.retry_after, Some(Duration::from_secs(90)));
        server.finish().unwrap();
    }
}

#[test]
fn local_transport_lists_models_with_scoped_oauth_token() {
    let (server, endpoint) = serve(
        200,
        r#"{"models":[{"slug":"model-a","display_name":"Model A","visibility":"list"}]}"#,
    );
    let api = Api::fixture(Session::fixture("fixture-access-token"), &endpoint);
    let models = api.catalog().unwrap();
    let (headers, body) = server.finish().unwrap();
    assert_eq!(models[0].id, "model-a");
    assert!(body.is_empty());
    assert!(headers[0].starts_with("GET /models "));
    assert!(
        headers
            .iter()
            .any(|line| line == "Authorization: Bearer fixture-access-token")
    );
}

#[test]
fn first_turn_loads_model_capacity_and_reuses_it_within_the_account_handle() {
    let (server, endpoint) = serve(
        200,
        r#"{"models":[{"slug":"model-a","display_name":"Model A","visibility":"list","context_window":500000,"supported_reasoning_levels":[{"effort":"low"},{"effort":"high"}]}]}"#,
    );
    let api = Api::fixture(Session::fixture("fixture-access-token"), &endpoint);
    *api.metadata.lock().unwrap() = None;
    let mut backend = OpenAi::with_api(api.clone(), "model-a".into()).unwrap();
    assert!(backend.known_limits().is_none());
    assert_eq!(backend.limits().unwrap().unwrap().context, 500_000);
    assert_eq!(backend.known_limits().unwrap().context, 500_000);
    let mut clone = OpenAi::with_api(api, "model-a".into()).unwrap();
    assert_eq!(clone.limits().unwrap().unwrap().context, 500_000);
    let (headers, _) = server.finish().unwrap();
    assert!(headers[0].starts_with("GET /models "));
}

#[test]
fn local_transport_streams_completed_tool_call_only_after_terminal_event() {
    let response = concat!(
        "data: {\"type\":\"response.output_item.done\",\"output_index\":0,\"item\":{\"type\":\"function_call\",\"namespace\":\"jecode\",\"call_id\":\"call_1\",\"name\":\"read\",\"arguments\":\"{\\\"path\\\":\\\"README.md\\\"}\"}}\n\n",
        "data: {\"type\":\"response.completed\",\"response\":{\"status\":\"completed\",\"output\":[{\"type\":\"function_call\",\"call_id\":\"call_1\",\"name\":\"read\",\"arguments\":\"{}\"}],\"usage\":{\"input_tokens\":10,\"output_tokens\":5}}}\n\n",
    );
    let (server, endpoint) = serve(200, response);
    let api = Api::fixture(Session::fixture("fixture-access-token"), &endpoint);
    let mut backend = OpenAi::with_api(api, "model-a".into()).unwrap();
    Backend::set_conversation(&mut backend, Some("session-1".into()));
    let messages = [Value::object([
        ("role", Value::string("user")),
        ("content", Value::string("Read README")),
    ])];
    let mut updates = Vec::new();
    let completion = backend
        .sample(&messages, true, None, None, &mut |update| {
            updates.push(update);
            Ok(())
        })
        .unwrap();
    let (headers, body) = server.finish().unwrap();
    assert!(headers[0].starts_with("POST /responses "));
    assert!(
        headers
            .iter()
            .any(|line| line == "Authorization: Bearer fixture-access-token")
    );
    assert!(headers.iter().any(|line| line == "session-id: session-1"));
    let request = crate::json::parse(&body).unwrap();
    assert_eq!(
        request.get("prompt_cache_key").and_then(Value::as_str),
        Some("session-1")
    );
    assert_eq!(request.get("store"), Some(&Value::Bool(false)));
    assert_eq!(request.get("stream"), Some(&Value::Bool(true)));
    assert_eq!(
        request.get("tools").and_then(Value::as_array).unwrap()[0]
            .get("type")
            .and_then(Value::as_str),
        Some("namespace")
    );
    assert_eq!(completion.calls.len(), 1);
    assert_eq!(completion.calls[0].arguments, "{\"path\":\"README.md\"}");
    assert_eq!(completion.usage.unwrap().input_tokens, Some(10));
    assert_eq!(
        completion
            .message
            .get("jecode_origin")
            .and_then(|origin| origin.get("account"))
            .and_then(Value::as_str),
        Some(backend.api.account().as_str())
    );
    assert!(matches!(updates[0], backend::Update::Waiting { .. }));
}

#[test]
fn local_transport_preserves_http_error_code_parameter_and_request_id() {
    let (server, endpoint) = serve(
        429,
        r#"{"error":{"code":"subscription_sharing_usage_limit_exceeded","param":"model","message":"limit"}}"#,
    );
    let api = Api::fixture(Session::fixture("fixture-access-token"), &endpoint);
    let backend = OpenAi::with_api(api, "model-a".into()).unwrap();
    let messages = [Value::object([
        ("role", Value::string("user")),
        ("content", Value::string("Hello")),
    ])];
    let failure = backend
        .sample(&messages, false, None, None, &mut |_| Ok(()))
        .err()
        .unwrap();
    server.finish().unwrap();
    assert_eq!(failure.kind, backend::FailureKind::Terminal);
    for part in [
        "HTTP 429",
        "subscription_sharing_usage_limit_exceeded",
        "param: model",
        "req_fixture",
    ] {
        assert!(
            failure.message.contains(part),
            "missing {part}: {}",
            failure.message
        );
    }
    assert!(failure.diagnostic.as_deref().unwrap().contains("\"error\""));
}

#[test]
fn admission_detail_body_is_retained_in_diagnostic() {
    let (server, endpoint) = serve(403, r#"{"detail":"region unavailable"}"#);
    let api = Api::fixture(Session::fixture("fixture-access-token"), &endpoint);
    let backend = OpenAi::with_api(api, "model-a".into()).unwrap();
    let messages = [Value::object([
        ("role", Value::string("user")),
        ("content", Value::string("Hello")),
    ])];
    let failure = backend
        .sample(&messages, false, None, None, &mut |_| Ok(()))
        .err()
        .unwrap();
    server.finish().unwrap();
    assert!(failure.message.contains("HTTP 403"));
    assert_eq!(
        failure.diagnostic.as_deref(),
        Some(r#"{"detail":"region unavailable"}"#)
    );
}
