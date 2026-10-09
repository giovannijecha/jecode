use crate::{json::Value, test_support::Directory, tools::Tools};
use std::fs;

const RECORD: &str = r#"{"action":"record","paths":["tests"],"reason":"Preserve existing tests"}"#;

fn fixture() -> (Directory, Tools) {
    let directory = Directory::new();
    fs::create_dir(directory.path().join("tests")).unwrap();
    fs::write(directory.path().join("tests/old.rs"), b"old test\r\n").unwrap();
    let tools = Tools::new(directory.path()).unwrap();
    tools.begin_request(1);
    (directory, tools)
}

fn journal(result: &Value) -> Vec<Value> {
    vec![
        Value::object([
            ("role", Value::string("system")),
            ("content", Value::string("fixture")),
        ]),
        Value::object([
            ("role", Value::string("user")),
            (
                "content",
                Value::string("Preserve existing tests; add a new test"),
            ),
        ]),
        Value::object([
            ("role", Value::string("assistant")),
            (
                "tool_calls",
                Value::Array(vec![Value::object([
                    ("id", Value::string("record-tests")),
                    (
                        "function",
                        Value::object([
                            ("name", Value::string("protect")),
                            ("arguments", Value::string(RECORD)),
                        ]),
                    ),
                ])]),
            ),
        ]),
        Value::object([
            ("role", Value::string("tool")),
            ("tool_call_id", Value::string("record-tests")),
            ("content", Value::string(result.encode())),
        ]),
        Value::object([
            ("role", Value::string("user")),
            (
                "content",
                Value::string("Format, preserving all existing tests"),
            ),
        ]),
    ]
}

#[test]
fn directory_declarations_retain_scope_across_requests_without_freezing_new_files_early() {
    let (directory, tools) = fixture();
    let recorded = tools.execute("protect", RECORD);
    assert!(recorded.get("error").is_none());
    let new = tools.execute(
        "write",
        r#"{"path":"tests/new.rs","content":"new test\r\n"}"#,
    );
    assert!(new.get("error").is_none());
    assert!(
        tools
            .execute(
                "write",
                r#"{"path":"tests/new.rs","content":"new test revised\r\n"}"#
            )
            .get("error")
            .is_none()
    );
    tools.begin_request(7);
    let blocked = tools.execute("protect", r#"{"action":"status","scope_exclusions":["tests/new.rs"],"reason":"It was created in the previous turn so formatting may edit it"}"#);
    assert!(blocked.get("error").is_some());
    let mutation = tools.execute("bash", r#"{"command":"printf bad > tests/new.rs"}"#);
    assert_eq!(
        mutation.get("outcome").and_then(Value::as_str),
        Some("not_started")
    );
    assert_eq!(
        fs::read(directory.path().join("tests/new.rs")).unwrap(),
        b"new test revised\r\n"
    );
    assert!(tools.execute("protect", RECORD).get("error").is_none());
    let mutation = tools.execute("bash", r#"{"command":"printf formatted > tests/new.rs"}"#);
    let entry = mutation
        .get("file_protections")
        .unwrap()
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| {
            entry
                .get("path")
                .and_then(Value::as_str)
                .is_some_and(|path| path.ends_with("new.rs"))
        })
        .unwrap();
    assert_eq!(entry.get("state").and_then(Value::as_str), Some("violated"));
    let token = entry
        .get("expected_current")
        .and_then(Value::as_str)
        .unwrap();
    let restored = tools.execute(
        "protect",
        &Value::object([
            ("action", Value::string("restore")),
            ("paths", Value::Array(vec![Value::string("tests/new.rs")])),
            ("expected_current", Value::string(token)),
        ])
        .encode(),
    );
    assert!(restored.get("error").is_none());
    assert_eq!(
        fs::read(directory.path().join("tests/new.rs")).unwrap(),
        b"new test revised\r\n"
    );
}

#[test]
fn current_and_legacy_receipts_recover_directory_scope_without_guessing_from_names() {
    for legacy in [false, true] {
        let (directory, tools) = fixture();
        let mut recorded = tools.execute("protect", RECORD);
        if legacy && let Value::Object(fields) = &mut recorded {
            fields.remove("scope_review");
        }
        fs::write(directory.path().join("tests/new.rs"), b"new\r\n").unwrap();
        let resumed = Tools::new(directory.path()).unwrap();
        resumed.restore_watches(&journal(&recorded)).unwrap();
        let excluded = resumed.execute("protect", r#"{"action":"status","scope_exclusions":["tests/new.rs"],"reason":"Try to forget the directory declaration"}"#);
        assert!(excluded.get("error").is_some(), "legacy={legacy}");
        assert!(resumed.execute("protect", RECORD).get("error").is_none());
        assert!(
            resumed
                .execute("write", r#"{"path":"tests/new.rs","content":"bad"}"#)
                .get("error")
                .is_some()
        );
    }
}

#[test]
fn newer_scope_release_is_durable_and_does_not_release_individual_baselines() {
    let (directory, tools) = fixture();
    let recorded = tools.execute("protect", RECORD);
    assert!(
        tools
            .execute(
                "protect",
                r#"{"action":"release","paths":["tests"],"reason":"No newer user decision"}"#
            )
            .get("error")
            .is_some()
    );
    fs::write(directory.path().join("tests/new.rs"), b"new\r\n").unwrap();
    tools.begin_request(4);
    let released = tools.execute("protect", r#"{"action":"release","paths":["tests"],"reason":"The newer user request permits editing newly added tests"}"#);
    assert_eq!(
        released.get("individual_baselines_unchanged"),
        Some(&Value::Bool(true))
    );
    let mut messages = journal(&recorded);
    messages.push(Value::object([
        ("role", Value::string("assistant")),
        ("tool_calls", Value::Array(vec![Value::object([
            ("id", Value::string("release-scope")),
            ("function", Value::object([("name", Value::string("protect")), ("arguments", Value::string(r#"{"action":"release","paths":["tests"],"reason":"User allows new tests"}"#))])),
        ])])),
    ]));
    messages.push(Value::object([
        ("role", Value::string("tool")),
        ("tool_call_id", Value::string("release-scope")),
        ("content", Value::string(released.encode())),
    ]));
    let resumed = Tools::new(directory.path()).unwrap();
    resumed.restore_watches(&messages).unwrap();
    assert!(resumed.execute("protect", r#"{"action":"status","scope_exclusions":["tests/new.rs"],"reason":"User permits editing new tests"}"#).get("error").is_none());
    assert!(
        resumed
            .execute("write", r#"{"path":"tests/new.rs","content":"allowed"}"#)
            .get("error")
            .is_none()
    );
    assert!(
        resumed
            .execute("write", r#"{"path":"tests/old.rs","content":"bad"}"#)
            .get("error")
            .is_some()
    );
    assert_eq!(
        fs::read(directory.path().join("tests/old.rs")).unwrap(),
        b"old test\r\n"
    );
}

#[test]
fn individual_release_leaves_other_directory_members_covered_even_without_active_old_baselines() {
    let (directory, tools) = fixture();
    tools.execute("protect", RECORD);
    tools.begin_request(4);
    let released = tools.execute("protect", r#"{"action":"release","paths":["tests/old.rs"],"reason":"The newer user permits editing this individual test"}"#);
    assert!(released.get("error").is_none());
    fs::write(directory.path().join("tests/new.rs"), b"new test\r\n").unwrap();
    tools.begin_request(7);
    let excluded = tools.execute("protect", r#"{"action":"status","scope_exclusions":["tests/old.rs"],"reason":"Its individual baseline was explicitly released"}"#);
    assert!(excluded.get("error").is_none());
    assert_eq!(
        excluded.get("scope_review").unwrap().get("reviewed"),
        Some(&Value::Bool(false))
    );
    let arguments = r#"{"path":"tests/new.rs","content":"bad"}"#;
    let blocked = tools.execute("write", arguments);
    assert_eq!(
        blocked.get("outcome").and_then(Value::as_str),
        Some("not_started")
    );
    let mut context = crate::context::Context::default();
    context.evidence.record(
        10,
        "write",
        &crate::json::parse(arguments).unwrap(),
        &blocked,
    );
    assert!(context.evidence.protection_problem().is_some());
    assert_eq!(
        fs::read(directory.path().join("tests/new.rs")).unwrap(),
        b"new test\r\n"
    );
    let registered = tools.execute("protect", r#"{"action":"record","paths":["tests/new.rs"],"reason":"Preserve the other existing test"}"#);
    assert!(registered.get("error").is_none());
    assert!(
        tools
            .execute("write", r#"{"path":"tests/old.rs","content":"permitted"}"#)
            .get("error")
            .is_none()
    );
    assert!(tools.execute("write", arguments).get("error").is_some());
}
