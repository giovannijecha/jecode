use super::*;
use crate::json;

fn tool(index: usize) -> Value {
    json::parse(&format!(
        r#"{{"id":"call_{index}","name":"bash","state":"done","text":"exit 0","arguments":{{"command":"node --test test/{index}.mjs","work":"Checks"}},"result":{{"exit_code":0,"stdout":"{}"}}}}"#,
        "x".repeat(4000)
    ))
    .unwrap()
}

fn full(id: &str) -> Value {
    Value::object([
        ("id", Value::string(id)),
        ("number", Value::number(1)),
        ("title", Value::string("Camera")),
        ("state", Value::string("returned")),
        ("workspace", Value::string("shared")),
        ("workspace_info", Value::object([("id", Value::string(id))])),
        ("directory", Value::string("C:/project")),
        ("instructions", Value::string("Implement the camera")),
        ("context", Value::string("Main context")),
        ("pending_instruction", Value::Null),
        ("sent_version", Value::number(2)),
        ("received_version", Value::number(2)),
        ("activity", Value::string("Returned")),
        (
            "updates",
            Value::Array(
                (0..15)
                    .map(|i| Value::string(format!("Update {i}")))
                    .collect(),
            ),
        ),
        ("tools", Value::Array((0..117).map(tool).collect())),
        (
            "log",
            Value::Array(vec![Value::object([("index", Value::number(0))])]),
        ),
        ("files", Value::Array(vec![Value::string("src/camera.js")])),
        (
            "checks",
            Value::Array(
                (0..30)
                    .map(|i| Value::string(format!("bash: exit {i}")))
                    .collect(),
            ),
        ),
        (
            "report",
            Value::object([
                ("version", Value::number(2)),
                ("text", Value::string("Camera done; 62 tests pass.")),
            ]),
        ),
        ("reviewed", Value::Bool(false)),
        ("error", Value::Null),
    ])
}

#[test]
fn returns_keep_report_and_evidence_without_collaborator_transcripts() {
    let receipt = Value::object([
        ("type", Value::string("collaboration")),
        ("kind", Value::string("reviewed")),
        ("id", Value::string("a1")),
        ("after_message", Value::number(40)),
        (
            "records",
            Value::Array(vec![full("a1"), full("a2"), full("a3")]),
        ),
    ]);
    let full_size = receipt.encode().len();
    let view = result("review", Some("a1"), receipt);
    let records = view.get("records").and_then(Value::as_array).unwrap();
    assert_eq!(records.len(), 1, "a review returns only its collaborator");
    assert!(view.get("after_message").is_none());
    let record = &records[0];
    assert_eq!(
        record
            .get("report")
            .and_then(|report| report.get("text"))
            .and_then(Value::as_str),
        Some("Camera done; 62 tests pass.")
    );
    assert_eq!(
        record.get("tool_calls").and_then(Value::as_usize),
        Some(117)
    );
    assert_eq!(
        record.get("checks_total").and_then(Value::as_usize),
        Some(30)
    );
    assert_eq!(
        record
            .get("checks")
            .and_then(Value::as_array)
            .unwrap()
            .len(),
        CHECKS
    );
    for hidden in ["tools", "log", "workspace_info", "instructions", "updates"] {
        assert!(record.get(hidden).is_none(), "{hidden} is not in the view");
    }
    assert!(view.encode().len() * 100 < full_size);
}

#[test]
fn inspect_adds_recent_work_as_short_lines() {
    let view = result("inspect", Some("a1"), full("a1"));
    let tools = view.get("recent_tools").and_then(Value::as_array).unwrap();
    assert_eq!(tools.len(), TOOLS);
    assert_eq!(
        tools.last().and_then(Value::as_str),
        Some("bash node --test test/116.mjs → exit 0")
    );
    assert_eq!(
        view.get("updates").and_then(Value::as_array).unwrap().len(),
        UPDATES
    );
    assert_eq!(
        view.get("instructions").and_then(Value::as_str),
        Some("Implement the camera")
    );
    assert!(view.encode().len() < 4096);
}

#[test]
fn recent_tools_state_what_each_call_recorded() {
    let line = |name: &str, state: &str, text: &str, arguments: &str| {
        tool_line(
            &json::parse(&format!(
                r#"{{"name":"{name}","state":"{state}","text":"{text}","arguments":{arguments}}}"#
            ))
            .unwrap(),
        )
    };
    assert_eq!(
        line("bash", "done", "timed out", r#"{"command":"cargo test"}"#),
        "bash cargo test → timed out"
    );
    assert_eq!(
        line("bash", "running", "", r#"{"command":"cargo build"}"#),
        "bash cargo build → running"
    );
    assert_eq!(
        line(
            "edit",
            "done",
            "changed 1 file; 1 failed",
            r#"{"files":[{"path":"a.rs","edits":[]},{"path":"b.rs","edits":[]}]}"#
        ),
        "edit a.rs, b.rs → changed 1 file; 1 failed"
    );
    assert_eq!(
        line("todo_list", "done", "complete", r#"{"action":"get"}"#),
        "todo_list get → complete"
    );
}

#[test]
fn running_collaborators_show_their_latest_update() {
    let mut running = full("a1");
    if let Value::Object(fields) = &mut running {
        fields.insert("report".into(), Value::Null);
        fields.insert("state".into(), Value::string("running"));
    }
    let view = result(
        "wait",
        None,
        Value::object([("records", Value::Array(vec![running]))]),
    );
    let record = &view.get("records").and_then(Value::as_array).unwrap()[0];
    assert_eq!(
        record.get("updates").and_then(Value::as_array).unwrap(),
        &[Value::string("Update 14")]
    );
}
