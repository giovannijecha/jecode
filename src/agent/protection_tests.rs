use super::*;
use crate::{
    json::Value,
    test_support::{Directory, HttpFixture, completion, tool_call},
};
use std::fs;

fn protect(id: &str, check: bool) -> Value {
    tool_call(
        id,
        "protect",
        Value::object([
            ("action", Value::string("record")),
            ("paths", Value::Array(vec![Value::string("protected")])),
            (
                "reason",
                Value::string("The user requires exact preservation"),
            ),
            ("require_check", Value::Bool(check)),
        ]),
    )
}

#[test]
fn read_and_false_final_cannot_complete_a_violated_contract_after_compaction_or_resume() {
    let directory = Directory::new();
    let home = Directory::new();
    let original = b"original\r\nlast";
    fs::write(directory.path().join("protected"), original).unwrap();
    let fixture = HttpFixture::new(vec![
        (200, completion("", vec![protect("protect", true)])),
        (
            200,
            completion(
                "",
                vec![tool_call(
                    "format",
                    "bash",
                    Value::object([
                        ("command", Value::string("printf formatted > protected")),
                        ("check", Value::Bool(true)),
                    ]),
                )],
            ),
        ),
        (
            200,
            completion(
                "",
                vec![tool_call(
                    "inspect",
                    "read",
                    Value::object([("path", Value::string("protected"))]),
                )],
            ),
        ),
        (200, completion("Everything is finished.", vec![])),
        (
            200,
            completion("I read the file, so everything is finished.", vec![]),
        ),
    ]);
    let mut first = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(directory.path()).unwrap(),
    );
    first.enable_sessions(home.path()).unwrap();
    let mut displayed = Vec::new();
    let error = first
        .run_turn(
            "Keep protected identical and verify changes.",
            &mut |event| {
                if let Event::Message { text } = event {
                    displayed.push(text);
                }
                Ok(())
            },
        )
        .unwrap_err();
    assert!(error.contains("Completion incomplete"), "{error}");
    assert!(displayed.is_empty(), "{displayed:?}");
    assert!(first.context.evidence.protection_problem().is_some());
    assert!(
        first
            .messages
            .lock()
            .unwrap()
            .iter()
            .any(|message| message.get("content").and_then(Value::as_str)
                == Some("Everything is finished."))
    );
    let requests = fixture.finish();
    let system = &requests[4]
        .body
        .get("messages")
        .unwrap()
        .as_array()
        .unwrap()[0];
    let review = system
        .get("content")
        .unwrap()
        .as_str()
        .unwrap()
        .split_once("Native completion state:\n")
        .unwrap()
        .1
        .lines()
        .next()
        .unwrap();
    let review = crate::json::parse(review).unwrap();
    assert_eq!(review.get("candidate_delivered"), Some(&Value::Bool(false)));
    assert!(
        review
            .get("native_blocker")
            .unwrap()
            .as_str()
            .unwrap()
            .contains("protected")
    );
    first.context.from = first.messages.lock().unwrap().len();
    first.context.summary =
        crate::context::memory::fixture("No protected-file issue listed by the model");
    first.save_session().unwrap();
    let id = first.sessions().unwrap().id();
    let expected = first.tools.protection_status(&Cancellation::default())[0]
        .get("expected_current")
        .and_then(Value::as_str)
        .unwrap()
        .to_owned();
    drop(first);
    let fixture = HttpFixture::new(vec![
        (
            200,
            completion(
                "",
                vec![tool_call(
                    "restore",
                    "protect",
                    Value::object([
                        ("action", Value::string("restore")),
                        ("paths", Value::Array(vec![Value::string("protected")])),
                        ("expected_current", Value::string(expected)),
                    ]),
                )],
            ),
        ),
        (
            200,
            completion(
                "",
                vec![tool_call(
                    "verify",
                    "bash",
                    Value::object([
                        (
                            "command",
                            Value::string("test \"$(wc -c < protected)\" -eq 14"),
                        ),
                        ("check", Value::Bool(true)),
                    ]),
                )],
            ),
        ),
        (200, completion("Restored and checked.", vec![])),
    ]);
    let mut resumed = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(directory.path()).unwrap(),
    );
    resumed.enable_sessions(home.path()).unwrap();
    resumed.resume(&id).unwrap();
    assert!(resumed.context.evidence.protection_problem().is_some());
    let projected = resumed
        .context
        .project(&resumed.messages.lock().unwrap(), 0);
    assert!(
        projected
            .iter()
            .any(|message| message.encode().contains("violated"))
    );
    resumed
        .run_turn(
            "Continue the same work and restore its incidental change.",
            &mut |_| Ok(()),
        )
        .unwrap();
    assert_eq!(
        fs::read(directory.path().join("protected")).unwrap(),
        original
    );
    assert!(resumed.context.evidence.protection_problem().is_none());
    resumed
        .context
        .validate(&resumed.messages.lock().unwrap())
        .unwrap();
    fixture.finish();
}

#[test]
fn a_contract_requiring_verification_cannot_finish_without_a_successful_fresh_check() {
    let directory = Directory::new();
    fs::write(directory.path().join("protected"), "original").unwrap();
    let fixture = HttpFixture::new(vec![
        (200, completion("", vec![protect("protect", true)])),
        (200, completion("Done without checking.", vec![])),
        (200, completion("Still done without checking.", vec![])),
    ]);
    let mut agent = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(directory.path()).unwrap(),
    );
    let error = agent
        .run_turn("Preserve protected and run verification.", &mut |_| Ok(()))
        .unwrap_err();
    assert!(error.contains("requires a successful check"), "{error}");
    assert_eq!(fixture.finish().len(), 3);
}

#[test]
fn a_mutating_passed_check_gets_a_separate_verification_instruction_before_delivery() {
    let directory = Directory::new();
    fs::write(directory.path().join("protected"), "original").unwrap();
    fs::write(directory.path().join("report"), "draft").unwrap();
    let command = |id, text| {
        tool_call(
            id,
            "bash",
            Value::object([
                ("command", Value::string(text)),
                ("check", Value::Bool(true)),
                ("watch", Value::Array(vec![Value::string("report")])),
            ]),
        )
    };
    let fixture = HttpFixture::new(vec![
        (200, completion("", vec![protect("protect", true)])),
        (
            200,
            completion(
                "",
                vec![command(
                    "repair-and-check",
                    "printf checked > report; test \"$(cat report)\" = checked",
                )],
            ),
        ),
        (200, completion("Done after the mutating check.", vec![])),
        (
            200,
            completion(
                "",
                vec![command(
                    "verify",
                    "test \"$(cat report)\" = checked; test \"$(cat protected)\" = original",
                )],
            ),
        ),
        (200, completion("Verified with a separate check.", vec![])),
    ]);
    let mut agent = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(directory.path()).unwrap(),
    );
    let mut displayed = Vec::new();
    agent
        .run_turn(
            "Repair the report; preserve protected and verify.",
            &mut |event| {
                if let Event::Message { text } = event {
                    displayed.push(text);
                }
                Ok(())
            },
        )
        .unwrap();
    assert_eq!(displayed, vec!["Verified with a separate check."]);
    assert_eq!(
        fs::read(directory.path().join("protected")).unwrap(),
        b"original"
    );
    assert!(agent.context.evidence.protection_problem().is_none());
    let value = agent.context.evidence.value();
    let checks = value.get("checks").unwrap().as_array().unwrap();
    assert_eq!(checks.len(), 2);
    assert_eq!(
        checks[0].get("check_status").and_then(Value::as_str),
        Some("passed")
    );
    assert_eq!(
        checks[0].get("after_last_tracked_change"),
        Some(&Value::Bool(false))
    );
    assert_eq!(
        checks[1].get("after_last_tracked_change"),
        Some(&Value::Bool(true))
    );
    let requests = fixture.finish();
    let recovery = requests[3].body.encode();
    assert!(
        recovery.contains("separate bash check:true command"),
        "{recovery}"
    );
    assert!(
        recovery.contains("without modifying project files"),
        "{recovery}"
    );
}

#[test]
fn a_read_only_contract_finishes_and_new_session_clears_protections() {
    let directory = Directory::new();
    fs::write(directory.path().join("protected"), "original").unwrap();
    let fixture = HttpFixture::new(vec![
        (200, completion("", vec![protect("protect", false)])),
        (200, completion("Read-only task finished.", vec![])),
    ]);
    let mut agent = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(directory.path()).unwrap(),
    );
    agent
        .run_turn("Inspect without changing protected.", &mut |_| Ok(()))
        .unwrap();
    assert!(agent.context.evidence.has_file_constraints());
    agent
        .start_new(agent.model().into(), Effort::Medium)
        .unwrap();
    assert!(!agent.context.evidence.has_file_constraints());
    assert!(
        agent
            .tools
            .protection_status(&Cancellation::default())
            .is_empty()
    );
    fixture.finish();
}

#[test]
fn final_native_comparison_catches_a_change_without_an_intervening_tool() {
    let directory = Directory::new();
    fs::write(directory.path().join("protected"), "original").unwrap();
    let fixture = HttpFixture::new(vec![
        (200, completion("", vec![protect("protect", false)])),
        (200, completion("Preserved.", vec![])),
        (200, completion("Preserved again.", vec![])),
    ]);
    let mut agent = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(directory.path()).unwrap(),
    );
    let error = agent
        .run_turn("Preserve protected.", &mut |event| {
            if let Event::ToolFinished { name, .. } = event
                && name == "protect"
            {
                fs::write(directory.path().join("protected"), "external change").unwrap();
            }
            Ok(())
        })
        .unwrap_err();
    assert!(error.contains("Completion incomplete"));
    assert_eq!(fixture.finish().len(), 3);
}
