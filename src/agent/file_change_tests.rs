use super::*;
use crate::test_support::{Directory, HttpFixture, completion, tool_call};
use std::fs;

fn reading(id: &str) -> Value {
    tool_call(
        id,
        "read",
        Value::object([("path", Value::string("protected"))]),
    )
}

fn command(id: &str, text: &str) -> Value {
    tool_call(
        id,
        "bash",
        Value::object([
            ("command", Value::string(text)),
            ("check", Value::Bool(true)),
        ]),
    )
}

#[test]
fn native_file_evidence_survives_compaction_resume_and_a_model_omitting_its_inspection() {
    let directory = Directory::new();
    let home = Directory::new();
    fs::write(directory.path().join("protected"), "original").unwrap();
    let fixture = HttpFixture::new(vec![
        (200, completion("", vec![reading("read")])),
        (
            200,
            completion("", vec![command("format", "printf formatted > protected")]),
        ),
        (200, completion("Done.", vec![])),
        (200, completion("Unresolved.", vec![])),
        (
            200,
            completion("", vec![command("restore", "printf original > protected")]),
        ),
        (
            200,
            completion(
                "",
                vec![command("check", "test \"$(cat protected)\" = original")],
            ),
        ),
        (200, completion("Verified.", vec![])),
    ]);
    let mut first = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(directory.path()).unwrap(),
    );
    first.enable_sessions(home.path()).unwrap();
    let mut notices = Vec::new();
    first
        .run_turn("Preserve protected exactly.", &mut |event| {
            if let Event::Maintenance { text } = event {
                notices.push(text);
            }
            Ok(())
        })
        .unwrap();
    assert!(
        notices
            .iter()
            .any(|text| text.contains("lack a subsequent read/write/edit"))
    );
    let messages = first.messages.lock().unwrap();
    let full =
        crate::json::parse(messages[5].get("content").and_then(Value::as_str).unwrap()).unwrap();
    assert_eq!(
        full.get("file_changes").unwrap().as_array().unwrap()[0]
            .get("first_observed")
            .unwrap()
            .get("history_reference")
            .and_then(Value::as_str),
        Some("history:3")
    );
    drop(messages);
    // A validated projection must retain owned facts even when its model memory omits the issue.
    first.context.from = first.messages.lock().unwrap().len();
    first.context.summary = crate::context::memory::fixture("Continue the task");
    first.save_session().unwrap();
    let id = first.sessions().unwrap().id();
    drop(first);
    let mut resumed = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(directory.path()).unwrap(),
    );
    resumed.enable_sessions(home.path()).unwrap();
    resumed.resume(&id).unwrap();
    assert!(resumed.context.evidence.needs_attention());
    resumed.run_turn("Continue.", &mut |_| Ok(())).unwrap();
    assert!(!resumed.context.evidence.needs_attention());
    assert_eq!(
        fs::read_to_string(directory.path().join("protected")).unwrap(),
        "original"
    );
    let saved = resumed.sessions().unwrap().snapshot();
    let restored = saved
        .messages
        .iter()
        .find_map(|message| {
            (message.get("tool_call_id").and_then(Value::as_str) == Some("restore")).then(|| {
                crate::json::parse(message.get("content").and_then(Value::as_str).unwrap()).unwrap()
            })
        })
        .unwrap();
    assert_eq!(
        restored.get("file_changes").unwrap().as_array().unwrap()[0]
            .get("restored_to_first_observed"),
        Some(&Value::Bool(true))
    );
    let requests = fixture.finish();
    assert!(
        requests[2]
            .body
            .encode()
            .contains("uninspected_file_change_count")
    );
    assert!(
        requests[3]
            .body
            .encode()
            .contains("uninspected_file_change_count")
    );
    assert!(requests[3].body.encode().contains("protected"));
    assert_eq!(requests.len(), 7);
}

#[test]
fn starting_a_new_session_drops_previous_file_observations() {
    let directory = Directory::new();
    fs::write(directory.path().join("protected"), "original").unwrap();
    let fixture = HttpFixture::new(vec![
        (200, completion("", vec![reading("read")])),
        (200, completion("Read.", vec![])),
        (
            200,
            completion("", vec![command("new", "printf changed > protected")]),
        ),
        (200, completion("Done.", vec![])),
    ]);
    let mut agent = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(directory.path()).unwrap(),
    );
    agent.run_turn("Read protected.", &mut |_| Ok(())).unwrap();
    agent
        .start_new("fixture/model".into(), Effort::Medium)
        .unwrap();
    agent.run_turn("Change it.", &mut |_| Ok(())).unwrap();
    assert_eq!(
        agent
            .tools
            .execute("bash", r#"{"command":"true"}"#)
            .get("file_tracking")
            .unwrap()
            .get("watched_files"),
        Some(&Value::number(0))
    );
    assert_eq!(fixture.finish().len(), 4);
}

#[test]
fn a_provisional_final_gets_one_review_and_its_repair_is_checked_before_display() {
    let directory = Directory::new();
    fs::write(directory.path().join("protected"), "original").unwrap();
    let fixture = HttpFixture::new(vec![
        (200, completion("", vec![reading("read")])),
        (
            200,
            completion("", vec![command("format", "printf changed > protected")]),
        ),
        (200, completion("Everything is preserved.", vec![])),
        (
            200,
            completion("", vec![command("restore", "printf original > protected")]),
        ),
        (
            200,
            completion(
                "",
                vec![command("check", "test \"$(cat protected)\" = original")],
            ),
        ),
        (200, completion("Restored and checked.", vec![])),
    ]);
    let mut agent = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(directory.path()).unwrap(),
    );
    let mut displayed = Vec::new();
    agent
        .run_turn("Keep protected byte-identical.", &mut |event| {
            if let Event::Message { text } = event {
                displayed.push(text);
            }
            Ok(())
        })
        .unwrap();
    assert_eq!(displayed, vec!["Restored and checked."]);
    assert_eq!(
        fs::read_to_string(directory.path().join("protected")).unwrap(),
        "original"
    );
    assert!(!agent.context.evidence.needs_attention());
    assert!(
        agent
            .messages
            .lock()
            .unwrap()
            .iter()
            .any(|message| message.get("content").and_then(Value::as_str)
                == Some("Everything is preserved."))
    );
    let requests = fixture.finish();
    assert_eq!(requests.len(), 6);
    assert!(
        requests[3]
            .body
            .encode()
            .contains(r#"\"candidate_delivered\":false"#)
    );
    assert!(
        requests[5]
            .body
            .encode()
            .contains("Native completion state")
    );
}
