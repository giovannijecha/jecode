#[cfg(windows)]
use crate::json::Value;
#[cfg(windows)]
use crate::test_support::Directory;

#[cfg(windows)]
#[test]
fn failed_bridge_start_keeps_unknown_outcome_and_output_references() {
    let root = Directory::new();
    let tools = super::tools(&root);
    let missing = root.path().join("missing-executable.exe");
    let result = tools.execute(
        "exec",
        &Value::object([
            ("executable", Value::string(missing.to_string_lossy())),
            ("tty", Value::Bool(true)),
            ("wait_ms", Value::number(0)),
        ])
        .encode(),
    );
    assert_eq!(
        result.get("outcome").and_then(Value::as_str),
        Some("unknown"),
        "{result:?}"
    );
    assert!(result.get("error").and_then(Value::as_str).is_some());
    assert!(result.get("process_id").is_none());
    for name in ["stdout_file", "stderr_file"] {
        let reference = result.get(name).and_then(Value::as_str).unwrap();
        let read = tools.execute(
            "read",
            &Value::object([("path", Value::string(reference))]).encode(),
        );
        assert!(
            read.get("error").is_none(),
            "{name} was not retained: {read:?}"
        );
    }
}
