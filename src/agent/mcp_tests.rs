use super::*;
use crate::{
    mcp::{AuthMethod, Connection, Store},
    permissions::Level,
    test_support::{Directory, HttpFixture, completion, tool_call},
};

fn rpc(id: usize, result: Value) -> (u16, Value) {
    (
        200,
        Value::object([
            ("jsonrpc", Value::string("2.0")),
            ("id", Value::number(id)),
            ("result", result),
        ]),
    )
}

fn discovery() -> (u16, Value) {
    rpc(
        1,
        Value::object([(
            "supportedVersions",
            Value::Array(vec![Value::string("2026-07-28")]),
        )]),
    )
}

fn tools() -> (u16, Value) {
    rpc(
        2,
        Value::object([(
            "tools",
            Value::Array(vec![Value::object([
                ("name", Value::string("create_issue")),
                (
                    "description",
                    Value::string("Create an issue in the chosen repository"),
                ),
                (
                    "inputSchema",
                    Value::object([
                        ("type", Value::string("object")),
                        ("additionalProperties", Value::Bool(true)),
                    ]),
                ),
            ])]),
        )]),
    )
}

#[test]
fn mcp_agent_discovers_and_calls_at_read_level_without_exposing_credentials() {
    let workspace = Directory::new();
    let home = Directory::new();
    let remote = HttpFixture::new(vec![
        discovery(),
        tools(),
        rpc(
            3,
            Value::object([
                (
                    "content",
                    Value::Array(vec![Value::object([
                        ("type", Value::string("text")),
                        (
                            "text",
                            Value::string(
                                "Created #7 using fixture-mcp-access and fixture-header-secret",
                            ),
                        ),
                    ])]),
                ),
                (
                    "structuredContent",
                    Value::object([("number", Value::number(7))]),
                ),
            ]),
        ),
    ]);
    let mut connection = Connection::draft("GitHub work", &remote.endpoint);
    connection.method = AuthMethod::Token;
    connection.token = "fixture-mcp-access".into();
    connection
        .headers
        .push(("X-Project-Key".into(), "fixture-header-secret".into()));
    let connection = Store::new(home.path().to_path_buf())
        .save(&connection)
        .unwrap();
    let model = HttpFixture::new(vec![
        (
            200,
            completion(
                "",
                vec![tool_call(
                    "discover",
                    "mcp",
                    Value::object([
                        ("action", Value::string("tools")),
                        ("connection", Value::string(&connection.id)),
                    ]),
                )],
            ),
        ),
        (
            200,
            completion(
                "",
                vec![tool_call(
                    "create",
                    "mcp",
                    Value::object([
                        ("action", Value::string("call")),
                        ("connection", Value::string(&connection.id)),
                        ("tool", Value::string("create_issue")),
                        (
                            "arguments",
                            Value::object([
                                ("owner", Value::string("example")),
                                ("title", Value::string("Fixture")),
                            ]),
                        ),
                        ("work", Value::string("Track issue")),
                    ]),
                )],
            ),
        ),
        (200, completion("Created issue #7.", vec![])),
    ]);
    let mut agent = Agent::new(
        OpenRouter::fixture(model.endpoint.clone()),
        Tools::new(workspace.path()).unwrap(),
    );
    agent.enable_sessions(home.path()).unwrap();
    agent.permissions().set_level(Level::Read).unwrap();
    let mut events = String::new();
    agent
        .run_turn("Create the fixture issue", &mut |event| {
            if let Event::ToolStarted { arguments, .. } = &event {
                assert_eq!(
                    arguments.get("connection_name").and_then(Value::as_str),
                    Some("GitHub work")
                );
            }
            events.push_str(&format!("{event:?}"));
            Ok(())
        })
        .unwrap();
    let requests = remote.finish();
    assert_eq!(requests.len(), 3);
    assert_eq!(
        requests
            .iter()
            .filter(
                |request| request.body.get("method").and_then(Value::as_str) == Some("tools/call")
            )
            .count(),
        1
    );
    assert!(requests.iter().all(|request| {
        request
            .headers
            .iter()
            .any(|header| header.eq_ignore_ascii_case("Authorization: Bearer fixture-mcp-access"))
    }));
    let final_model_request = model.finish().pop().unwrap().body.encode();
    assert!(final_model_request.contains("structuredContent"));
    assert!(final_model_request.contains("[redacted]"));
    let archive = agent.archive().document().encode();
    for secret in ["fixture-mcp-access", "fixture-header-secret"] {
        assert!(!final_model_request.contains(secret));
        assert!(!events.contains(secret));
        assert!(!archive.contains(secret));
    }
}

#[test]
fn suspended_or_cancelled_mcp_does_not_dispatch() {
    let workspace = Directory::new();
    let home = Directory::new();
    let store = Store::new(home.path().to_path_buf());
    let mut connection = Connection::draft("Disabled", "http://127.0.0.1:1/mcp");
    connection.enabled = false;
    let connection = store.save(&connection).unwrap();
    let mut tools = Tools::new(workspace.path()).unwrap();
    tools.configure_mcp(home.path(), crate::redact::Redactor::empty());
    let arguments = Value::object([
        ("action", Value::string("call")),
        ("connection", Value::string(&connection.id)),
        ("tool", Value::string("create_issue")),
    ])
    .encode();
    let result = tools.execute("mcp", &arguments);
    assert!(
        result
            .get("error")
            .and_then(Value::as_str)
            .unwrap()
            .contains("suspended")
    );
    store.set_enabled(&connection.id, true).unwrap();
    let cancel = Cancellation::default();
    cancel.cancel();
    let result = crate::mcp::execute(
        Some(home.path()),
        &crate::json::parse(&arguments).unwrap(),
        &cancel,
        &crate::redact::Redactor::empty(),
    );
    assert_eq!(
        result.get("outcome").and_then(Value::as_str),
        Some("not_executed")
    );
}

#[test]
fn mcp_tools_and_call_reuse_the_same_legacy_server_session() {
    let workspace = Directory::new();
    let home = Directory::new();
    let remote = HttpFixture::streaming(vec![
        crate::test_support::Response::Json(
            200,
            Value::object([
                ("jsonrpc", Value::string("2.0")),
                ("id", Value::number(1)),
                ("error", Value::object([("code", Value::number(-32601))])),
            ]),
        ),
        crate::test_support::Response::Http(
            200,
            vec![("Mcp-Session-Id".into(), "fixture-session".into())],
            rpc(
                2,
                Value::object([
                    ("protocolVersion", Value::string("2025-11-25")),
                    ("capabilities", Value::object([])),
                    (
                        "serverInfo",
                        Value::object([
                            ("name", Value::string("stateful")),
                            ("version", Value::string("1")),
                        ]),
                    ),
                ]),
            )
            .1,
        ),
        crate::test_support::Response::Json(202, Value::object([])),
        crate::test_support::Response::Json(
            200,
            rpc(3, Value::object([("tools", Value::Array(vec![]))])).1,
        ),
        crate::test_support::Response::Json(
            200,
            rpc(4, Value::object([("content", Value::Array(vec![]))])).1,
        ),
    ]);
    let connection = Store::new(home.path().to_path_buf())
        .save(&Connection::draft("Stateful", &remote.endpoint))
        .unwrap();
    let mut tools = Tools::new(workspace.path()).unwrap();
    tools.configure_mcp(home.path(), crate::redact::Redactor::empty());
    let result = tools.execute(
        "mcp",
        &Value::object([
            ("action", Value::string("tools")),
            ("connection", Value::string(&connection.id)),
        ])
        .encode(),
    );
    assert!(result.get("error").is_none(), "{result:?}");
    let result = tools.execute(
        "mcp",
        &Value::object([
            ("action", Value::string("call")),
            ("connection", Value::string(&connection.id)),
            ("tool", Value::string("continue_session")),
        ])
        .encode(),
    );
    assert!(result.get("error").is_none(), "{result:?}");
    let requests = remote.finish();
    assert_eq!(requests.len(), 5);
    for request in &requests[2..] {
        assert!(
            request
                .headers
                .iter()
                .any(|header| header.eq_ignore_ascii_case("Mcp-Session-Id: fixture-session"))
        );
    }
    assert_eq!(
        requests[4].body.get("method").and_then(Value::as_str),
        Some("tools/call")
    );
}

#[test]
fn mcp_call_accepts_encoded_arguments_and_bounds_large_results() {
    let workspace = Directory::new();
    let home = Directory::new();
    let image = "QUJD".repeat(4096);
    let remote = HttpFixture::new(vec![
        discovery(),
        tools(),
        rpc(
            3,
            Value::object([(
                "content",
                Value::Array(vec![
                    Value::object([
                        ("type", Value::string("text")),
                        ("text", Value::string("fixture line\n".repeat(10_000))),
                    ]),
                    Value::object([
                        ("type", Value::string("image")),
                        ("mimeType", Value::string("image/png")),
                        ("data", Value::string(&image)),
                    ]),
                ]),
            )]),
        ),
    ]);
    let connection = Store::new(home.path().to_path_buf())
        .save(&Connection::draft("Large", &remote.endpoint))
        .unwrap();
    let mut tools = Tools::new(workspace.path()).unwrap();
    tools.configure_mcp(home.path(), crate::redact::Redactor::empty());
    let result = tools.execute(
        "mcp",
        &Value::object([
            ("action", Value::string("call")),
            ("connection", Value::string(&connection.id)),
            ("tool", Value::string("create_issue")),
            ("arguments", Value::string(r#"{"owner":"example"}"#)),
        ])
        .encode(),
    );
    assert!(result.get("error").is_none(), "{result:?}");
    assert_eq!(result.get("truncated"), Some(&Value::Bool(true)));
    let inline = result.encode();
    assert!(inline.len() <= 64 * 1024);
    assert!(!inline.contains(&image));
    let reference = result.get("result_file").and_then(Value::as_str).unwrap();
    let saved = tools.execute(
        "read",
        &Value::object([("path", Value::string(reference))]).encode(),
    );
    assert!(saved.encode().contains("fixture line"), "{saved:?}");
    let requests = remote.finish();
    assert_eq!(
        requests[2]
            .body
            .get("params")
            .and_then(|params| params.get("arguments")),
        Some(&Value::object([("owner", Value::string("example"))]))
    );
}

#[test]
fn corrupt_mcp_configuration_leaves_sessions_working_and_reaches_the_model() {
    let workspace = Directory::new();
    let home = Directory::new();
    std::fs::write(home.path().join("mcp.json"), "{broken").unwrap();
    let mut agent = Agent::new(
        OpenRouter::fixture("http://127.0.0.1:9/chat/completions".into()),
        Tools::new(workspace.path()).unwrap(),
    );
    agent.enable_sessions(home.path()).unwrap();
    let error = agent.configure_mcp(home.path()).unwrap_err();
    assert!(error.contains("mcp.json"), "{error}");
    let result = agent.tools.execute(
        "mcp",
        &Value::object([("action", Value::string("list"))]).encode(),
    );
    assert_eq!(
        result.get("outcome").and_then(Value::as_str),
        Some("failed")
    );
    assert!(result.get("error").and_then(Value::as_str).is_some());
}
