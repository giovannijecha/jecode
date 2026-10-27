use crate::json::Value;
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

#[test]
fn invalid_protect_rejects_terminal_controls_before_delivery() {
    let root = Directory::new();
    let tools = super::tools(&root);
    let started = super::execute(&tools, super::fixture(), &[], &[]);
    let id = super::text(&started, "process_id").to_owned();
    let mut output = super::text(&started, "stdout").to_owned();
    super::collect_until(&tools, &id, &mut output, "READY");
    for action in ["write", "resize", "interrupt"] {
        let mut fields = vec![
            ("action", Value::string(action)),
            ("process_id", Value::string(&id)),
            ("protect", Value::Array(vec![Value::number(7)])),
        ];
        if action == "write" {
            fields.push(("text", Value::string("incorrect")));
        } else if action == "resize" {
            fields.push(("cols", Value::number(101)));
            fields.push(("rows", Value::number(37)));
        }
        let invalid = tools.execute(
            "process",
            &Value::Object(
                fields
                    .into_iter()
                    .map(|(key, value)| (key.to_string(), value))
                    .collect(),
            )
            .encode(),
        );
        assert!(
            super::text(&invalid, "error").contains("protect"),
            "{invalid:?}"
        );
    }
    let sent = super::action(&tools, &id, "write", &[("text", Value::string("correct"))]);
    output.push_str(super::text(&sent, "stdout"));
    let entered = super::action(&tools, &id, "write", &[("key", Value::string("Enter"))]);
    output.push_str(super::text(&entered, "stdout"));
    let ended = super::finish(&tools, &id, &mut output);
    assert_eq!(super::text(&ended, "status"), "exited");
    assert!(output.contains("INPUT:\"correct\""), "{output}");
    assert!(
        !output.contains("INPUT:\"incorrect"),
        "invalid input was sent: {output}"
    );
}

#[cfg(unix)]
#[test]
fn interrupt_captures_preimage_before_signal_handler_changes_file() {
    use crate::cancel::Cancellation;
    use crate::checkpoints::session::Session;
    use std::io::Read;
    use std::path::Path;

    let home = Directory::new();
    let root = Directory::new();
    std::fs::write(root.path().join("notes.txt"), b"before\n").unwrap();
    let mut tools = super::tools(&root);
    let session = Session::offline(home.path(), root.path());
    session.begin("conversation-a");
    tools.configure_checkpoints(session);
    let started = super::execute(
        &tools,
        Path::new("/bin/sh"),
        &[
            "-c",
            "printf 'READY\\n'; trap 'printf changed > notes.txt; exit 130' INT; while :; do sleep 1; done",
        ],
        &[],
    );
    let id = super::text(&started, "process_id").to_owned();
    let mut output = super::text(&started, "stdout").to_owned();
    super::collect_until(&tools, &id, &mut output, "READY");
    let interrupted = super::action(
        &tools,
        &id,
        "interrupt",
        &[("protect", Value::Array(vec![Value::string("notes.txt")]))],
    );
    output.push_str(super::text(&interrupted, "stdout"));
    let checkpoint = interrupted.get("checkpoint").unwrap();
    assert_eq!(
        checkpoint.get("status").and_then(Value::as_str),
        Some("saved")
    );
    let reference = checkpoint.get("ref").and_then(Value::as_str).unwrap();
    let ended = super::finish(&tools, &id, &mut output);
    assert_eq!(super::text(&ended, "status"), "exited");
    assert_eq!(
        std::fs::read(root.path().join("notes.txt")).unwrap(),
        b"changed"
    );
    let store = tools.checkpoints().unwrap().store().unwrap();
    let saved = store.load(reference, &Cancellation::default()).unwrap();
    let mut bytes = Vec::new();
    saved
        .reader(saved.snapshot.entries.get("notes.txt").unwrap())
        .unwrap()
        .read_to_end(&mut bytes)
        .unwrap();
    assert_eq!(bytes, b"before\n");
}
