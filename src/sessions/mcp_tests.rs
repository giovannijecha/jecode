use crate::{
    json::Value,
    test_support::{Directory, tool_call},
};

/// The recovered result of an MCP request interrupted by a crash.
fn recovered(action: &str) -> Value {
    let workspace = Directory::new();
    let home = Directory::new();
    let store = crate::sessions::Store::new(home.path().to_path_buf(), workspace.path()).unwrap();
    let mut document = crate::sessions::Document::new(
        workspace.path().to_path_buf(),
        "fixture/model".into(),
        crate::effort::Effort::Default,
    )
    .unwrap();
    document.messages = vec![
        Value::object([
            ("role", Value::string("system")),
            ("content", Value::string("Fixture system")),
        ]),
        Value::object([
            ("role", Value::string("user")),
            ("content", Value::string("Create an issue")),
        ]),
        Value::object([
            ("role", Value::string("assistant")),
            (
                "tool_calls",
                Value::Array(vec![tool_call(
                    "remote-started",
                    "mcp",
                    Value::object([
                        ("action", Value::string(action)),
                        ("connection", Value::string("missing-connection")),
                        ("tool", Value::string("create_issue")),
                    ]),
                )]),
            ),
        ]),
    ];
    document.pending = crate::sessions::Pending {
        active: true,
        tool: Some("remote-started".into()),
        ..Default::default()
    };
    let lease = store.acquire(&document.id).unwrap();
    lease.save(&document).unwrap();
    drop(lease);
    let (handle, _) =
        crate::sessions::Handle::open(store, &document.id, crate::redact::Redactor::empty())
            .unwrap();
    let recovered = handle.snapshot();
    assert!(!recovered.pending.active);
    let result = recovered
        .messages
        .last()
        .unwrap()
        .get("content")
        .and_then(Value::as_str)
        .unwrap();
    let result = crate::json::parse(result).unwrap();
    assert_eq!(
        result.get("outcome").and_then(Value::as_str),
        Some("unknown")
    );
    result
}

#[test]
fn recovered_remote_call_reports_uncertainty_without_replaying_it() {
    let result = recovered("call");
    let error = result.get("error").and_then(Value::as_str).unwrap();
    assert!(error.contains("Check the service"));
    assert!(error.contains("cannot undo remote changes"));
}

#[test]
fn recovered_discovery_is_reported_as_read_only() {
    let result = recovered("tools");
    let error = result.get("error").and_then(Value::as_str).unwrap();
    assert!(error.contains("read-only MCP request"), "{error}");
    assert!(!error.contains("remote changes"));
}
