use super::recovery_tests::{action, create, repository};
use super::*;
use crate::{
    backend::openrouter::OpenRouter,
    test_support::{Directory, HttpFixture, completion, tool_call},
};
use std::{fs, io::Read};

#[test]
fn shared_work_diff_exposes_recorded_edits_without_claiming_a_baseline() {
    let project = Directory::new();
    let home = Directory::new();
    fs::write(project.path().join("notes.txt"), "before\n").unwrap();
    let fixture = HttpFixture::new(vec![
        (
            200,
            completion(
                "",
                vec![tool_call(
                    "change",
                    "write",
                    Value::object([
                        ("path", Value::string("notes.txt")),
                        ("content", Value::string("after\n")),
                    ]),
                )],
            ),
        ),
        (200, completion("Changed notes", vec![])),
    ]);
    let mut parent = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(project.path()).unwrap(),
    );
    parent.enable_sessions(home.path()).unwrap();
    parent.agent_tool(&create("shared")).unwrap();
    parent
        .collaboration()
        .wait(Some("a1"), Duration::from_secs(10), &parent.cancellation)
        .unwrap();
    let summary = parent.agent_tool(&action("diff")).unwrap();
    assert_eq!(
        summary.get("source").and_then(Value::as_str),
        Some("recorded_tool_changes")
    );
    assert_eq!(
        summary.get("files").and_then(Value::as_array).unwrap(),
        &[Value::string("notes.txt")]
    );
    let mut request = action("diff");
    if let Value::Object(fields) = &mut request {
        fields.insert("path".into(), Value::string("notes.txt"));
        fields.insert("limit".into(), Value::number(24));
    }
    let page = parent.agent_tool(&request).unwrap();
    let mut text = page
        .get("diff")
        .and_then(Value::as_str)
        .unwrap()
        .to_string();
    let next = page.get("next_offset").and_then(Value::as_usize).unwrap();
    assert!(next > 0);
    if let Value::Object(fields) = &mut request {
        fields.insert("offset".into(), Value::number(next));
        fields.insert("limit".into(), Value::number(65_536));
    }
    fs::write(project.path().join("notes.txt"), "later main edit\n").unwrap();
    let rest = parent.agent_tool(&request).unwrap();
    text.push_str(rest.get("diff").and_then(Value::as_str).unwrap());
    assert!(text.contains("-before\n"), "{text}");
    assert!(text.contains("+after\n"), "{text}");
    assert!(!text.contains("later main edit"));
    assert!(rest.get("review_token").is_none());
    assert_eq!(rest.get("complete"), Some(&Value::Bool(false)));
    fixture.finish();
}

#[test]
fn child_write_keeps_a_recoverable_preimage() {
    let project = Directory::new();
    let home = Directory::new();
    fs::write(project.path().join("notes.txt"), "before\n").unwrap();
    let fixture = HttpFixture::new(vec![
        (
            200,
            completion(
                "",
                vec![tool_call(
                    "write-1",
                    "write",
                    Value::object([
                        ("path", Value::string("notes.txt")),
                        ("content", Value::string("after\n")),
                    ]),
                )],
            ),
        ),
        (200, completion("Written", vec![])),
    ]);
    let mut parent = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(project.path()).unwrap(),
    );
    parent.enable_sessions(home.path()).unwrap();
    parent.agent_tool(&create("shared")).unwrap();
    let records = parent
        .collaboration()
        .wait(
            Some("a1"),
            Duration::from_secs(5),
            &crate::cancel::Cancellation::default(),
        )
        .unwrap();
    let tool = &records[0].get("tools").and_then(Value::as_array).unwrap()[0];
    let checkpoint = tool.get("result").unwrap().get("checkpoint").unwrap();
    assert_eq!(
        checkpoint.get("status").and_then(Value::as_str),
        Some("saved")
    );
    let reference = checkpoint.get("ref").and_then(Value::as_str).unwrap();
    let handle = parent.sessions().unwrap();
    let child_home = handle
        .store()
        .bucket()
        .join("collaboration")
        .join(handle.id())
        .join("child-home");
    let session = crate::checkpoints::session::Session::offline(&child_home, project.path());
    let loaded = session
        .store()
        .unwrap()
        .load(reference, &crate::cancel::Cancellation::default())
        .unwrap();
    let mut original = String::new();
    loaded
        .reader(loaded.snapshot.entries.get("notes.txt").unwrap())
        .unwrap()
        .read_to_string(&mut original)
        .unwrap();
    assert_eq!(original, "before\n");
    assert_eq!(
        fs::read_to_string(project.path().join("notes.txt")).unwrap(),
        "after\n"
    );
    assert_eq!(fixture.finish().len(), 2);
}

#[test]
fn integration_captures_selected_main_project_preimage() {
    let (_temp, project) = repository();
    let home = Directory::new();
    let fixture = HttpFixture::new(vec![
        (
            200,
            completion(
                "",
                vec![tool_call(
                    "write-1",
                    "write",
                    Value::object([
                        ("path", Value::string("file.txt")),
                        ("content", Value::string("child\n")),
                    ]),
                )],
            ),
        ),
        (200, completion("Changed file", vec![])),
    ]);
    let mut parent = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(&project).unwrap(),
    );
    parent.enable_sessions(home.path()).unwrap();
    parent.enable_checkpoints(home.path());
    parent.agent_tool(&create("isolated")).unwrap();
    parent
        .collaboration()
        .wait(
            Some("a1"),
            Duration::from_secs(5),
            &crate::cancel::Cancellation::default(),
        )
        .unwrap();
    let review = parent.agent_tool(&action("diff")).unwrap();
    let token = review.get("review_token").and_then(Value::as_str).unwrap();
    let integrated = parent
        .agent_tool(&Value::object([
            ("action", Value::string("integrate")),
            ("id", Value::string("a1")),
            ("paths", Value::Array(vec![Value::string("file.txt")])),
            ("expected_review_token", Value::string(token)),
        ]))
        .unwrap();
    let checkpoint = integrated.get("checkpoint").unwrap();
    assert_eq!(
        checkpoint.get("status").and_then(Value::as_str),
        Some("saved")
    );
    let reference = checkpoint.get("ref").and_then(Value::as_str).unwrap();
    let loaded = parent
        .tools
        .checkpoints()
        .unwrap()
        .store()
        .unwrap()
        .load(reference, &crate::cancel::Cancellation::default())
        .unwrap();
    let mut original = String::new();
    loaded
        .reader(loaded.snapshot.entries.get("file.txt").unwrap())
        .unwrap()
        .read_to_string(&mut original)
        .unwrap();
    assert_eq!(original, "source\n");
    assert_eq!(
        fs::read_to_string(project.join("file.txt")).unwrap(),
        "child\n"
    );
    parent.agent_tool(&action("close")).unwrap();
    assert_eq!(fixture.finish().len(), 2);
}

#[test]
fn interrupted_integration_capture_leaves_project_untouched() {
    let (_temp, project) = repository();
    let home = Directory::new();
    let fixture = HttpFixture::new(vec![
        (
            200,
            completion(
                "",
                vec![tool_call(
                    "write-1",
                    "write",
                    Value::object([
                        ("path", Value::string("file.txt")),
                        ("content", Value::string("child\n")),
                    ]),
                )],
            ),
        ),
        (200, completion("Changed file", vec![])),
    ]);
    let mut parent = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(&project).unwrap(),
    );
    parent.enable_sessions(home.path()).unwrap();
    parent.enable_checkpoints(home.path());
    parent.agent_tool(&create("isolated")).unwrap();
    parent
        .collaboration()
        .wait(
            Some("a1"),
            Duration::from_secs(5),
            &crate::cancel::Cancellation::default(),
        )
        .unwrap();
    let review = parent.agent_tool(&action("diff")).unwrap();
    let token = review
        .get("review_token")
        .and_then(Value::as_str)
        .unwrap()
        .to_owned();
    // Hold the file's capture lease so the integration waits inside capture.
    let store = std::sync::Arc::clone(parent.tools.checkpoints().unwrap().store().unwrap());
    let lease = store
        .leases
        .acquire_cancellable(&["file.txt"], &crate::cancel::Cancellation::default())
        .unwrap();
    let cancellation = parent.cancellation();
    let integrating = std::thread::spawn(move || {
        let result = parent.agent_tool(&Value::object([
            ("action", Value::string("integrate")),
            ("id", Value::string("a1")),
            ("paths", Value::Array(vec![Value::string("file.txt")])),
            ("expected_review_token", Value::string(token)),
        ]));
        (parent, result)
    });
    std::thread::sleep(Duration::from_millis(200));
    cancellation.cancel();
    let (parent, result) = integrating.join().unwrap();
    drop(lease);
    assert_eq!(result.unwrap_err(), "Operation cancelled");
    assert_eq!(
        fs::read_to_string(project.join("file.txt")).unwrap(),
        "source\n"
    );
    drop(parent);
    assert_eq!(fixture.finish().len(), 2);
}
