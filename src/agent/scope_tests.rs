use super::*;
use crate::test_support::{Directory, HttpFixture, completion, tool_call};
use std::fs;

#[test]
fn a_refused_scope_operation_cannot_be_reported_complete_and_survives_resume() {
    let home = Directory::new();
    let directory = Directory::new();
    fs::write(directory.path().join("keep"), "original").unwrap();
    let fixture = HttpFixture::new(vec![
        (
            200,
            completion(
                "",
                vec![tool_call(
                    "record",
                    "protect",
                    Value::object([
                        ("action", Value::string("record")),
                        ("paths", Value::Array(vec![Value::string("keep")])),
                        ("reason", Value::string("Keep original bytes")),
                    ]),
                )],
            ),
        ),
        (200, completion("Registered", vec![])),
        (
            200,
            completion(
                "",
                vec![tool_call(
                    "attempt",
                    "bash",
                    Value::object([("command", Value::string("printf ran >> executions.txt"))]),
                )],
            ),
        ),
        (200, completion("Finished", vec![])),
        (200, completion("Still finished", vec![])),
    ]);
    let mut first = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(directory.path()).unwrap(),
    );
    first.enable_sessions(home.path()).unwrap();
    first.run_turn("Preserve keep.", &mut |_| Ok(())).unwrap();
    let error = first
        .run_turn("Run the requested operation.", &mut |_| Ok(()))
        .unwrap_err();
    assert!(error.contains("scope"), "{error}");
    assert!(!directory.path().join("executions.txt").exists());
    let id = first.sessions().unwrap().id();
    drop(first);
    fixture.finish();
    let mut resumed = Agent::new(
        OpenRouter::fixture("http://127.0.0.1:1/unused".into()),
        Tools::new(directory.path()).unwrap(),
    );
    resumed.enable_sessions(home.path()).unwrap();
    resumed.resume(&id).unwrap();
    assert!(
        resumed
            .context
            .evidence
            .protection_problem()
            .is_some_and(|error| error.contains("scope"))
    );
    assert_eq!(
        fs::read_to_string(directory.path().join("keep")).unwrap(),
        "original"
    );
}
