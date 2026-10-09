use crate::{json::Value, test_support::Directory, tools::Tools};
use std::fs;

#[test]
fn contract_regression_empty_scope_options_preserve_registration_and_release_guards() {
    let directory = Directory::new();
    fs::write(directory.path().join("keep.bin"), b"original").unwrap();
    let tools = Tools::new(directory.path()).unwrap();
    let registered = tools.execute(
        "protect",
        r#"{"action":"record","paths":["keep.bin"],"reason":"Keep the user's bytes","scope_exclusions":[],"expected_current":"","require_check":false}"#,
    );
    assert!(registered.get("error").is_none(), "{}", registered.encode());
    let release = r#"{"action":"release","paths":["keep.bin"],"reason":"The newer request permits editing this file","scope_exclusions":[],"expected_current":"","require_check":false}"#;
    let refused = tools.execute("protect", release);
    assert!(refused.get("error").is_some());
    let blocked = tools.execute("write", r#"{"path":"keep.bin","content":"wrong"}"#);
    assert!(blocked.get("error").is_some());
    assert_eq!(
        fs::read(directory.path().join("keep.bin")).unwrap(),
        b"original"
    );
    tools.begin_request(7);
    let released = tools.execute("protect", release);
    assert!(released.get("error").is_none(), "{}", released.encode());
    let written = tools.execute("write", r#"{"path":"keep.bin","content":"authorized"}"#);
    assert!(written.get("error").is_none(), "{}", written.encode());
    assert_eq!(
        fs::read(directory.path().join("keep.bin")).unwrap(),
        b"authorized"
    );
}

#[test]
fn contract_regression_empty_status_defaults_do_not_require_a_scope_decision() {
    let directory = Directory::new();
    let tools = Tools::new(directory.path()).unwrap();
    let status = tools.execute(
        "protect",
        r#"{"action":"status","paths":[],"reason":"","scope_exclusions":[],"expected_current":"","require_check":false}"#,
    );
    assert!(status.get("error").is_none(), "{}", status.encode());
}

#[test]
fn contract_regression_nonempty_exclusions_are_rejected_by_other_actions() {
    let directory = Directory::new();
    fs::write(directory.path().join("keep.bin"), b"original").unwrap();
    fs::write(directory.path().join("other.bin"), b"editable").unwrap();
    let tools = Tools::new(directory.path()).unwrap();
    let refused = tools.execute(
        "protect",
        r#"{"action":"record","paths":["keep.bin"],"reason":"Keep original bytes","scope_exclusions":["other.bin"]}"#,
    );
    assert!(refused.get("error").is_some());
    let status = tools.execute("protect", r#"{"action":"status"}"#);
    assert!(
        status
            .get("file_protections")
            .and_then(Value::as_array)
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        fs::read(directory.path().join("keep.bin")).unwrap(),
        b"original"
    );
}

fn record(tools: &Tools, paths: &[&str]) -> Value {
    tools.execute(
        "protect",
        &Value::object([
            ("action", Value::string("record")),
            (
                "paths",
                Value::Array(paths.iter().map(|path| Value::string(*path)).collect()),
            ),
            (
                "reason",
                Value::string("Preserve the user's existing tests"),
            ),
        ])
        .encode(),
    )
}

#[test]
fn a_new_request_reviews_added_files_before_any_project_operation_runs() {
    let directory = Directory::new();
    fs::create_dir(directory.path().join("tests")).unwrap();
    fs::write(directory.path().join("tests/original.rs"), "old test").unwrap();
    fs::write(directory.path().join("editable.rs"), "editable source").unwrap();
    let tools = Tools::new(directory.path()).unwrap();
    tools.begin_request(1);
    assert!(record(&tools, &["tests"]).get("error").is_none());
    fs::write(directory.path().join("tests/new.rs"), "new test\r\n").unwrap();
    tools.begin_request(7);
    let blocked = tools.execute("bash", r#"{"command":"printf ran >> executions.txt"}"#);
    assert_eq!(
        blocked.get("outcome").and_then(Value::as_str),
        Some("not_started")
    );
    assert!(!directory.path().join("executions.txt").exists());
    assert!(
        blocked
            .get("scope_review")
            .unwrap()
            .get("related_unregistered_files")
            .unwrap()
            .as_array()
            .unwrap()
            .iter()
            .any(|path| path
                .as_str()
                .is_some_and(|path| path.replace('\\', "/") == "tests/new.rs"))
    );
    let blocked = tools.execute("write", r#"{"path":"editable.rs","content":"changed"}"#);
    assert_eq!(
        blocked.get("outcome").and_then(Value::as_str),
        Some("not_started")
    );
    assert_eq!(
        fs::read_to_string(directory.path().join("editable.rs")).unwrap(),
        "editable source"
    );

    assert!(record(&tools, &["tests"]).get("error").is_none());
    assert!(
        tools
            .execute("write", r#"{"path":"editable.rs","content":"changed"}"#)
            .get("error")
            .is_none()
    );
    assert!(
        tools
            .execute("write", r#"{"path":"tests/new.rs","content":"changed"}"#)
            .get("error")
            .is_some()
    );
    let run = tools.execute("bash", r#"{"command":"printf ran >> executions.txt"}"#);
    assert_eq!(run.get("exit_code").and_then(Value::as_usize), Some(0));
    assert_eq!(
        fs::read_to_string(directory.path().join("executions.txt")).unwrap(),
        "ran"
    );
}

#[test]
fn reviewing_candidates_does_not_make_them_immutable_without_a_constraint() {
    let directory = Directory::new();
    fs::create_dir(directory.path().join("tests")).unwrap();
    fs::write(directory.path().join("tests/original.rs"), "old test").unwrap();
    let tools = Tools::new(directory.path()).unwrap();
    tools.begin_request(1);
    record(&tools, &["tests/original.rs"]);
    fs::write(directory.path().join("tests/new.rs"), "editable new test").unwrap();
    tools.begin_request(7);
    let status = tools.execute("protect", r#"{"action":"status"}"#);
    assert_eq!(
        status.get("scope_review").unwrap().get("reviewed"),
        Some(&Value::Bool(false))
    );
    let blocked = tools.execute("write", r#"{"path":"tests/new.rs","content":"wrong"}"#);
    assert_eq!(
        blocked.get("outcome").and_then(Value::as_str),
        Some("not_started")
    );
    let excluded = tools.execute("protect", r#"{"action":"status","scope_exclusions":["tests/new.rs"],"reason":"The current request explicitly permits changing this new test"}"#);
    assert_eq!(
        excluded.get("scope_review").unwrap().get("reviewed"),
        Some(&Value::Bool(true))
    );
    assert!(
        tools
            .execute("write", r#"{"path":"tests/new.rs","content":"allowed"}"#)
            .get("error")
            .is_none()
    );
    assert_eq!(
        fs::read_to_string(directory.path().join("tests/new.rs")).unwrap(),
        "allowed"
    );
}

#[test]
fn exclusions_require_a_reason_and_cannot_waive_a_registered_baseline() {
    let directory = Directory::new();
    fs::create_dir(directory.path().join("tests")).unwrap();
    fs::write(directory.path().join("tests/keep.rs"), "original").unwrap();
    fs::write(directory.path().join("tests/other.rs"), "editable").unwrap();
    let tools = Tools::new(directory.path()).unwrap();
    record(&tools, &["tests/keep.rs"]);
    tools.begin_request(7);
    assert!(
        tools
            .execute(
                "protect",
                r#"{"action":"status","scope_exclusions":["tests/other.rs"]}"#
            )
            .get("error")
            .is_some()
    );
    assert!(tools.execute("protect", r#"{"action":"status","scope_exclusions":["tests/keep.rs"],"reason":"Try to waive the old baseline"}"#).get("error").is_some());
    assert_eq!(
        fs::read_to_string(directory.path().join("tests/keep.rs")).unwrap(),
        "original"
    );
}

#[test]
fn incomplete_related_inventories_are_explicit_and_bounded() {
    let directory = Directory::new();
    fs::create_dir(directory.path().join("tests")).unwrap();
    fs::write(directory.path().join("tests/original.rs"), "old test").unwrap();
    let tools = Tools::new(directory.path()).unwrap();
    record(&tools, &["tests/original.rs"]);
    for index in 0..90 {
        fs::write(
            directory.path().join(format!("tests/new-{index}.rs")),
            "new test",
        )
        .unwrap();
    }
    tools.begin_request(7);
    let status = tools.execute("protect", r#"{"action":"status"}"#);
    let review = status.get("scope_review").unwrap();
    assert_eq!(
        review.get("related_inventory_complete"),
        Some(&Value::Bool(false))
    );
    assert!(
        review
            .get("related_unregistered_files")
            .unwrap()
            .as_array()
            .unwrap()
            .len()
            <= 64
    );
}
