use super::*;
use crate::{redact::Redactor, test_support::Directory};

#[test]
fn model_sees_only_the_endpoint_origin_and_users_lose_only_the_query() {
    let endpoint = "https://user:fixture-secret@mcp.example.com:8443/v1/fixture-path?key=k#f";
    assert_eq!(origin(endpoint), "https://mcp.example.com:8443");
    assert_eq!(origin("http://127.0.0.1:9/mcp"), "http://127.0.0.1:9");
    assert_eq!(
        without_query(endpoint),
        "https://user:fixture-secret@mcp.example.com:8443/v1/fixture-path"
    );
}

#[test]
fn only_credential_like_or_long_header_values_become_secrets() {
    let mut connection = Connection::draft("Headers", "https://mcp.example.com/mcp");
    connection.headers = vec![
        ("X-MCP-Readonly".into(), "true".into()),
        ("X-Api-Key".into(), "short-key".into()),
        ("Cookie".into(), "sid=abc".into()),
        ("X-Tenant".into(), "fixture-tenant-identifier".into()),
    ];
    let redactor = Redactor::empty();
    connection.protect(&redactor);
    assert_eq!(
        redactor.text(r#"{"ok":true} short-key sid=abc fixture-tenant-identifier"#),
        r#"{"ok":true} [redacted] [redacted] [redacted]"#
    );
}

fn call_result(result: Value) -> Value {
    Value::object([
        ("outcome", Value::string("completed")),
        ("tool", Value::string("fixture")),
        ("result", result),
    ])
}

#[test]
fn binary_blocks_become_placeholders_and_the_full_result_is_saved() {
    let directory = Directory::new();
    let outputs = output::Store::new(directory.path().join("output"), Redactor::empty());
    let data = "QUJD".repeat(1000);
    let value = bounded::bound(
        call_result(Value::object([(
            "content",
            Value::Array(vec![
                Value::object([
                    ("type", Value::string("image")),
                    ("mimeType", Value::string("image/png")),
                    ("data", Value::string(&data)),
                ]),
                Value::object([
                    ("type", Value::string("resource")),
                    (
                        "resource",
                        Value::object([
                            ("uri", Value::string("file:///fixture.bin")),
                            ("blob", Value::string("QUJD")),
                        ]),
                    ),
                ]),
            ]),
        )])),
        Some(&outputs),
    );
    let inline = value.encode();
    assert!(!inline.contains(&data));
    assert!(inline.contains("[omitted image/png data: 3000 bytes; see result_file]"));
    assert!(inline.contains("[omitted application/octet-stream data: 3 bytes"));
    assert!(value.get("truncated").is_none());
    let reference = value.get("result_file").and_then(Value::as_str).unwrap();
    let saved = std::fs::read_to_string(outputs.resolve(reference).unwrap()).unwrap();
    assert!(saved.contains(&data));
}

#[test]
fn oversized_text_keeps_a_bounded_preview() {
    let text = "line \"quoted\"\n".repeat(20_000);
    let value = bounded::bound(
        call_result(Value::object([
            (
                "content",
                Value::Array(vec![Value::object([
                    ("type", Value::string("text")),
                    ("text", Value::string(&text)),
                ])]),
            ),
            ("isError", Value::Bool(false)),
        ])),
        None,
    );
    assert!(value.encode().len() <= crate::tools::OUTPUT_LIMIT);
    assert_eq!(value.get("truncated"), Some(&Value::Bool(true)));
    assert!(value.get("result_file").is_none());
    let result = value.get("result").unwrap();
    assert_eq!(result.get("isError"), Some(&Value::Bool(false)));
    assert!(summary(&value).starts_with("line"));
    let small = call_result(Value::object([("content", Value::Array(vec![]))]));
    assert_eq!(bounded::bound(small.clone(), None), small);
}

#[test]
fn oversized_discovery_keeps_exact_schemas_in_a_readable_result_file() {
    let directory = Directory::new();
    let outputs = output::Store::new(directory.path().join("output"), Redactor::empty());
    let mut catalog = crate::mcp::Catalog::default();
    catalog.server.instructions = Some("instructions ".repeat(6000));
    catalog.tools.push(Tool {
        name: "large_schema".into(),
        title: None,
        description: "A tool with a large schema".into(),
        input_schema: Value::object([
            ("type", Value::string("object")),
            ("description", Value::string("schema ".repeat(12_000))),
        ]),
        annotations: None,
    });
    let connection = Connection::draft("Large", "https://mcp.example.com/mcp");
    let full = discovered(&connection, &catalog, &Value::object([]));
    let inline = bounded::bound(full.clone(), Some(&outputs));
    assert!(inline.encode().len() <= crate::tools::OUTPUT_LIMIT);
    assert_eq!(inline.get("truncated"), Some(&Value::Bool(true)));
    let reference = inline.get("result_file").and_then(Value::as_str).unwrap();
    let saved = std::fs::read_to_string(outputs.resolve(reference).unwrap()).unwrap();
    assert_eq!(crate::json::parse(&saved).unwrap(), full);
    assert!(summary(&inline).contains("result_file"));
}

#[test]
fn inline_budget_includes_the_call_envelope() {
    let full = call_result(Value::string("x".repeat(crate::tools::OUTPUT_LIMIT - 8)));
    assert!(full.get("result").unwrap().encode().len() < crate::tools::OUTPUT_LIMIT);
    let inline = bounded::bound(full, None);
    assert!(inline.encode().len() <= crate::tools::OUTPUT_LIMIT);
    assert_eq!(inline.get("truncated"), Some(&Value::Bool(true)));
}

#[test]
fn summaries_show_the_page_against_the_total_and_plain_failures_say_error() {
    let page = Value::object([
        ("tools", Value::Array(vec![Value::object([]); 8])),
        ("total", Value::number(120)),
    ]);
    assert_eq!(summary(&page), "8 of 120 tools");
    let complete = Value::object([
        ("tools", Value::Array(vec![Value::object([]); 2])),
        ("total", Value::number(2)),
    ]);
    assert_eq!(summary(&complete), "2 tools");
    let failed = Value::object([
        ("error", Value::string("Not Found")),
        ("outcome", Value::string("failed")),
    ]);
    assert_eq!(
        crate::events::tool_summary("mcp", &failed),
        "error: Not Found"
    );
    assert_eq!(crate::events::tool_summary("mcp", &complete), "2 tools");
}

#[test]
fn a_call_waiting_for_a_busy_connection_can_be_cancelled() {
    let home = Directory::new();
    let store = Store::new(home.path().to_path_buf());
    let saved = store
        .save(&Connection::draft("Busy", "http://127.0.0.1:9/mcp"))
        .unwrap();
    let runtime = std::sync::Arc::new(Runtime::default());
    let shared = runtime.client(home.path(), &saved).unwrap();
    let busy = shared.lock().unwrap();
    let cancel = Cancellation::default();
    let waiting = {
        let (runtime, cancel, home) = (runtime.clone(), cancel.clone(), home.path().to_owned());
        std::thread::spawn(move || {
            execute_with(
                &runtime,
                Some(&home),
                &Value::object([
                    ("action", Value::string("call")),
                    ("connection", Value::string("Busy")),
                    ("tool", Value::string("fixture")),
                ]),
                &cancel,
                &Redactor::empty(),
                None,
            )
        })
    };
    std::thread::sleep(Duration::from_millis(100));
    assert!(!waiting.is_finished());
    cancel.cancel();
    let result = waiting.join().unwrap();
    drop(busy);
    assert_eq!(
        result.get("outcome").and_then(Value::as_str),
        Some("not_executed")
    );
    assert_eq!(
        crate::events::tool_summary("mcp", &result),
        "Cancelled · call was not sent"
    );
}
