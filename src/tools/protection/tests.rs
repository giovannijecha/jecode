use crate::{json::Value, test_support::Directory, tools::Tools};
use std::fs;

fn call(tools: &Tools, action: &str, paths: &[&str]) -> Value {
    tools.execute(
        "protect",
        &Value::object([
            ("action", Value::string(action)),
            (
                "paths",
                Value::Array(paths.iter().map(|path| Value::string(*path)).collect()),
            ),
            (
                "reason",
                Value::string("Keep the user's existing files unchanged"),
            ),
        ])
        .encode(),
    )
}

fn entry(result: &Value) -> &Value {
    &result.get("file_protections").unwrap().as_array().unwrap()[0]
}

fn restore(tools: &Tools, path: &str, token: &str) -> Value {
    tools.execute(
        "protect",
        &Value::object([
            ("action", Value::string("restore")),
            ("paths", Value::Array(vec![Value::string(path)])),
            ("expected_current", Value::string(token)),
        ])
        .encode(),
    )
}

#[test]
fn attaching_history_does_not_repeat_unchanged_baselines_but_new_changes_still_report() {
    let directory = Directory::new();
    fs::write(directory.path().join("keep.txt"), "original").unwrap();
    let tools = Tools::new(directory.path()).unwrap();
    let registered = call(&tools, "record", &["keep.txt"]);
    tools.record_file_history(3, &registered);
    let read = tools.execute("read", r#"{"path":"keep.txt"}"#);
    assert!(
        read.get("file_protections")
            .and_then(Value::as_array)
            .is_none_or(|entries| entries.is_empty())
    );
    fs::write(directory.path().join("keep.txt"), "changed").unwrap();
    let changed = tools.execute("read", r#"{"path":"keep.txt"}"#);
    assert_eq!(
        entry(&changed).get("state").and_then(Value::as_str),
        Some("violated")
    );
    assert_eq!(
        entry(&changed)
            .get("history_reference")
            .and_then(Value::as_str),
        Some("history:3")
    );
}

#[test]
fn protected_binary_bytes_restore_exactly_without_retyping_or_rebasing() {
    let directory = Directory::new();
    let original = b"a\r\nb\0\xff";
    fs::write(directory.path().join("keep.bin"), original).unwrap();
    let tools = Tools::new(directory.path()).unwrap();
    let registered = call(&tools, "record", &["keep.bin"]);
    assert!(registered.get("error").is_none(), "{registered:?}");
    assert_eq!(
        entry(&registered).get("state").and_then(Value::as_str),
        Some("preserved")
    );
    fs::write(directory.path().join("keep.bin"), b"changed").unwrap();
    let repeated = call(&tools, "record", &["keep.bin"]);
    assert_eq!(
        entry(&repeated).get("baseline"),
        entry(&registered).get("baseline")
    );
    assert_eq!(
        entry(&repeated).get("state").and_then(Value::as_str),
        Some("violated")
    );
    let expected = entry(&repeated)
        .get("expected_current")
        .and_then(Value::as_str)
        .unwrap();
    let restored = restore(&tools, "keep.bin", expected);
    assert!(restored.get("error").is_none(), "{restored:?}");
    assert_eq!(
        fs::read(directory.path().join("keep.bin")).unwrap(),
        original
    );
    assert_eq!(
        entry(&restored).get("state").and_then(Value::as_str),
        Some("preserved")
    );
}

#[test]
fn restoration_refuses_a_newer_version_and_handles_a_missing_file() {
    let directory = Directory::new();
    fs::write(directory.path().join("keep.txt"), b"original\r\n").unwrap();
    let tools = Tools::new(directory.path()).unwrap();
    call(&tools, "record", &["keep.txt"]);
    fs::write(directory.path().join("keep.txt"), b"first change").unwrap();
    let status = call(&tools, "status", &[]);
    let expected = entry(&status)
        .get("expected_current")
        .and_then(Value::as_str)
        .unwrap();
    fs::write(directory.path().join("keep.txt"), b"newer user change").unwrap();
    assert!(restore(&tools, "keep.txt", expected).get("error").is_some());
    assert_eq!(
        fs::read(directory.path().join("keep.txt")).unwrap(),
        b"newer user change"
    );
    fs::remove_file(directory.path().join("keep.txt")).unwrap();
    let status = call(&tools, "status", &[]);
    assert_eq!(
        entry(&status)
            .get("expected_current")
            .and_then(Value::as_str),
        Some("missing")
    );
    assert!(
        restore(&tools, "keep.txt", "missing")
            .get("error")
            .is_none()
    );
    assert_eq!(
        fs::read(directory.path().join("keep.txt")).unwrap(),
        b"original\r\n"
    );
}

#[test]
fn file_tools_and_same_request_release_cannot_waive_the_contract() {
    let directory = Directory::new();
    fs::write(directory.path().join("keep.txt"), "original").unwrap();
    let tools = Tools::new(directory.path()).unwrap();
    tools.begin_request(1);
    call(&tools, "record", &["keep.txt"]);
    let write = tools.execute("write", r#"{"path":"keep.txt","content":"new"}"#);
    assert!(write.get("error").is_some());
    assert!(
        tools
            .execute(
                "edit",
                r#"{"path":"keep.txt","old_text":"original","new_text":"new"}"#
            )
            .get("error")
            .is_some()
    );
    assert!(
        call(&tools, "release", &["keep.txt"])
            .get("error")
            .is_some()
    );
    fs::write(directory.path().join("keep.txt"), "changed").unwrap();
    tools.execute("read", r#"{"path":"keep.txt"}"#);
    assert_eq!(
        entry(&call(&tools, "status", &[]))
            .get("state")
            .and_then(Value::as_str),
        Some("violated")
    );
    tools.begin_request(8);
    assert!(
        call(&tools, "release", &["keep.txt"])
            .get("error")
            .is_none()
    );
    assert!(
        tools
            .execute("write", r#"{"path":"keep.txt","content":"allowed"}"#)
            .get("error")
            .is_none()
    );
}

#[test]
fn directory_protection_includes_existing_files_and_bash_reports_violations() {
    let directory = Directory::new();
    fs::create_dir(directory.path().join("tests")).unwrap();
    fs::write(directory.path().join("tests/a.rs"), "a").unwrap();
    fs::write(directory.path().join("tests/b.rs"), "b").unwrap();
    let tools = Tools::new(directory.path()).unwrap();
    let registered = call(&tools, "record", &["tests"]);
    assert_eq!(
        registered
            .get("protected_file_count")
            .and_then(Value::as_usize),
        Some(2)
    );
    let result = tools.execute(
        "bash",
        r#"{"command":"printf changed > tests/a.rs","check":true}"#,
    );
    assert_eq!(
        result.get("check_status").and_then(Value::as_str),
        Some("passed")
    );
    let states = result.get("file_protections").unwrap().as_array().unwrap();
    assert!(
        states
            .iter()
            .any(|entry| entry.get("state").and_then(Value::as_str) == Some("violated"))
    );
    fs::write(directory.path().join("tests/new.rs"), "new").unwrap();
    assert!(
        tools
            .execute(
                "write",
                r#"{"path":"tests/new.rs","content":"allowed new test"}"#
            )
            .get("error")
            .is_none()
    );
}

#[test]
fn missing_baseline_is_unknown_and_cannot_be_recreated_from_changed_content() {
    let directory = Directory::new();
    fs::write(directory.path().join("keep.txt"), "original").unwrap();
    let tools = Tools::new(directory.path()).unwrap();
    let result = call(&tools, "record", &["keep.txt"]);
    let id = entry(&result)
        .get("snapshot_id")
        .and_then(Value::as_str)
        .unwrap();
    fs::remove_file(
        directory
            .path()
            .join(".jecode-output")
            .join(format!("{id}.baseline")),
    )
    .unwrap();
    fs::write(directory.path().join("keep.txt"), "changed").unwrap();
    let status = call(&tools, "record", &["keep.txt"]);
    assert_eq!(
        entry(&status).get("state").and_then(Value::as_str),
        Some("unknown")
    );
    assert!(
        restore(&tools, "keep.txt", "missing")
            .get("error")
            .is_some()
    );
    assert_eq!(
        fs::read_to_string(directory.path().join("keep.txt")).unwrap(),
        "changed"
    );
}

#[test]
fn invalid_outside_scope_is_rejected_before_registering_any_file() {
    let directory = Directory::new();
    fs::write(directory.path().join("keep.txt"), "original").unwrap();
    let outside = Directory::new();
    fs::write(outside.path().join("outside.txt"), "outside").unwrap();
    let tools = Tools::new(directory.path()).unwrap();
    let result = call(
        &tools,
        "record",
        &[
            "keep.txt",
            &outside.path().join("outside.txt").to_string_lossy(),
        ],
    );
    assert!(result.get("error").is_some());
    assert!(
        call(&tools, "status", &[])
            .get("file_protections")
            .unwrap()
            .as_array()
            .unwrap()
            .is_empty()
    );
}
