use super::*;
use crate::{
    backend::openrouter::OpenRouter,
    test_support::{Directory, HttpFixture, completion, tool_call},
};

pub(super) fn create() -> Value {
    Value::object([
        ("action", Value::string("create")),
        (
            "tasks",
            Value::Array(vec![Value::object([
                ("title", Value::string("Inspect source")),
                ("instructions", Value::string("Report the source state")),
                ("context", Value::string("The project is a fixture")),
                ("workspace", Value::string("shared")),
            ])]),
        ),
    ])
}

#[test]
fn collaborator_returns_and_followup_keeps_versions_and_review_explicit() {
    let project = Directory::new();
    let home = Directory::new();
    let fixture = HttpFixture::new(vec![
        (200, completion("First report", vec![])),
        (200, completion("Corrected report", vec![])),
    ]);
    let mut parent = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(project.path()).unwrap(),
    );
    parent.enable_sessions(home.path()).unwrap();
    let created = parent.agent_tool(&create()).unwrap();
    assert_eq!(
        created.get("created").and_then(Value::as_array).unwrap()[0]
            .get("id")
            .and_then(Value::as_str),
        Some("a1")
    );
    let observer = parent.collaboration();
    let first = observer
        .wait(
            Some("a1"),
            Duration::from_secs(5),
            &crate::cancel::Cancellation::default(),
        )
        .unwrap();
    let listing = parent.sessions().unwrap().store().list().unwrap();
    assert_eq!(
        listing.sessions.len(),
        1,
        "child session must stay outside main session picker"
    );
    assert_eq!(
        first[0].get("state").and_then(Value::as_str),
        Some("returned")
    );
    assert_eq!(
        first[0].get("received_version").and_then(Value::as_usize),
        Some(1)
    );
    assert_eq!(first[0].get("reviewed"), Some(&Value::Bool(false)));
    let context = parent.request_environment();
    assert!(context.contains("Only you, the main agent, review"));
    assert!(context.contains("action=review, id, report_version"));
    assert!(context.contains(
        "a1 Inspect source: returned, received assignment v1, report v1, awaiting main-agent review"
    ));
    parent
        .agent_tool(&Value::object([
            ("action", Value::string("update")),
            ("id", Value::string("a1")),
            (
                "instructions",
                Value::string("Report again after correction"),
            ),
        ]))
        .unwrap();
    let second = observer
        .wait(
            Some("a1"),
            Duration::from_secs(5),
            &crate::cancel::Cancellation::default(),
        )
        .unwrap();
    assert_eq!(
        second[0].get("state").and_then(Value::as_str),
        Some("returned")
    );
    assert_eq!(
        second[0].get("received_version").and_then(Value::as_usize),
        Some(2)
    );
    assert_eq!(
        second[0]
            .get("report")
            .and_then(|v| v.get("text"))
            .and_then(Value::as_str),
        Some("Corrected report")
    );
    assert_eq!(second[0].get("reviewed"), Some(&Value::Bool(false)));
    parent
        .agent_tool(&Value::object([
            ("action", Value::string("review")),
            ("id", Value::string("a1")),
            ("report_version", Value::number(2)),
            ("note", Value::string("Checked source")),
        ]))
        .unwrap();
    assert_eq!(
        observer.snapshot()[0].get("reviewed"),
        Some(&Value::Bool(true))
    );
    assert!(
        parent
            .request_environment()
            .contains("report v2, reviewed; close when finished")
    );
    let receipts = observer.events_since(0).1;
    assert_eq!(
        receipts
            .iter()
            .filter_map(|v| v.get("kind").and_then(Value::as_str))
            .collect::<Vec<_>>(),
        vec![
            "created", "received", "returned", "sent", "received", "returned", "reviewed"
        ]
    );
    observer.close("a1").unwrap();
    assert!(observer.snapshot().is_empty());
    assert!(
        !parent
            .request_environment()
            .contains("Active collaborators")
    );
    assert_eq!(fixture.finish().len(), 2);
}

#[test]
fn resume_recovers_records_and_marks_unfinished_work_unknown() {
    let project = Directory::new();
    let home = Directory::new();
    let fixture = HttpFixture::new(vec![(200, completion("Seeded", vec![]))]);
    let mut parent = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(project.path()).unwrap(),
    );
    parent.enable_sessions(home.path()).unwrap();
    parent
        .run_turn("Start project work", &mut |_| Ok(()))
        .unwrap();
    let workspace = crate::collaboration::workspace::Workspace::prepare(
        project.path(),
        &home.path().join("owned"),
        "a1",
        Mode::Shared,
    )
    .unwrap();
    let record = Value::object([
        ("id", Value::string("a1")),
        ("number", Value::number(1)),
        ("title", Value::string("Unfinished")),
        ("state", Value::string("running")),
        ("workspace_info", workspace.value()),
        ("sent_version", Value::number(2)),
        ("received_version", Value::number(1)),
        ("instructions", Value::string("Old")),
    ]);
    let event = Value::object([
        ("type", Value::string("collaboration")),
        ("kind", Value::string("received")),
        ("sequence", Value::number(1)),
        ("id", Value::string("a1")),
        ("records", Value::Array(vec![record])),
        (
            "after_message",
            Value::number(parent.archive().messages.lock().unwrap().len()),
        ),
    ]);
    parent
        .sessions()
        .unwrap()
        .append_event(&parent.archive(), event)
        .unwrap();
    let id = parent.sessions().unwrap().id();
    parent.save_session().unwrap();
    drop(parent);
    let mut resumed = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(project.path()).unwrap(),
    );
    resumed.enable_sessions(home.path()).unwrap();
    resumed.resume(&id).unwrap();
    let record = &resumed.collaboration().snapshot()[0];
    assert_eq!(record.get("state").and_then(Value::as_str), Some("unknown"));
    assert_eq!(
        record.get("received_version").and_then(Value::as_usize),
        Some(1)
    );
    assert_eq!(
        resumed
            .collaboration()
            .events_since(0)
            .1
            .last()
            .unwrap()
            .get("kind")
            .and_then(Value::as_str),
        Some("unknown")
    );
    assert_eq!(fixture.finish().len(), 1);
}

#[test]
fn update_during_child_command_is_received_before_its_next_request() {
    let project = Directory::new();
    let home = Directory::new();
    let fixture = HttpFixture::new(vec![
        (
            200,
            completion(
                "Running a check.",
                vec![tool_call(
                    "check",
                    "bash",
                    Value::object([("command", Value::string("sleep 0.6"))]),
                )],
            ),
        ),
        (200, completion("Final corrected report", vec![])),
    ]);
    let mut parent = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(project.path()).unwrap(),
    );
    parent.enable_sessions(home.path()).unwrap();
    parent
        .permissions()
        .set_level(crate::permissions::Level::Full)
        .unwrap();
    parent.agent_tool(&create()).unwrap();
    let observer = parent.collaboration();
    let started = std::time::Instant::now();
    while observer.snapshot()[0]
        .get("tools")
        .and_then(Value::as_array)
        .is_none_or(|tools| tools.is_empty())
    {
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "child command did not start"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(
        observer.snapshot()[0]
            .get("updates")
            .and_then(Value::as_array)
            .unwrap()[0]
            .as_str(),
        Some("Running a check.")
    );
    parent
        .agent_tool(&Value::object([
            ("action", Value::string("update")),
            ("id", Value::string("a1")),
            ("instructions", Value::string("Use the corrected scope")),
        ]))
        .unwrap();
    let state = observer.snapshot();
    assert_eq!(
        state[0].get("sent_version").and_then(Value::as_usize),
        Some(2)
    );
    assert_eq!(
        state[0].get("received_version").and_then(Value::as_usize),
        Some(1)
    );
    let finished = observer
        .wait(
            Some("a1"),
            Duration::from_secs(5),
            &crate::cancel::Cancellation::default(),
        )
        .unwrap();
    assert_eq!(
        finished[0].get("state").and_then(Value::as_str),
        Some("returned")
    );
    assert_eq!(
        finished[0]
            .get("report")
            .and_then(|v| v.get("version"))
            .and_then(Value::as_usize),
        Some(2)
    );
    assert_eq!(
        finished[0]
            .get("report")
            .and_then(|v| v.get("text"))
            .and_then(Value::as_str),
        Some("Running a check.\n\nFinal corrected report")
    );
    let receipts = observer.events_since(0).1;
    let sent = receipts
        .iter()
        .position(|value| value.get("kind").and_then(Value::as_str) == Some("sent"))
        .unwrap();
    let received = receipts
        .iter()
        .rposition(|value| value.get("kind").and_then(Value::as_str) == Some("received"))
        .unwrap();
    assert!(sent < received);
    observer.stop_all();
    let requests = fixture.finish();
    assert_eq!(requests.len(), 2);
    assert!(
        requests[1]
            .body
            .encode()
            .contains("Use the corrected scope")
    );
}

#[test]
fn child_cannot_execute_agent_tool_even_if_backend_returns_one() {
    let project = Directory::new();
    let home = Directory::new();
    let fixture = HttpFixture::new(vec![
        (
            200,
            completion(
                "",
                vec![
                    tool_call(
                        "delegate",
                        "agent",
                        Value::object([("action", Value::string("list"))]),
                    ),
                    tool_call(
                        "inspect",
                        "read",
                        Value::object([("path", Value::string("source.txt"))]),
                    ),
                ],
            ),
        ),
        (200, completion("Handled the tool results", vec![])),
    ]);
    std::fs::write(project.path().join("source.txt"), "owned source").unwrap();
    let mut parent = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(project.path()).unwrap(),
    );
    parent.enable_sessions(home.path()).unwrap();
    parent.agent_tool(&create()).unwrap();
    let observer = parent.collaboration();
    let result = observer
        .wait(
            Some("a1"),
            Duration::from_secs(5),
            &crate::cancel::Cancellation::default(),
        )
        .unwrap();
    assert_eq!(
        result[0].get("state").and_then(Value::as_str),
        Some("returned")
    );
    observer.stop_all();
    let requests = fixture.finish();
    assert_eq!(requests.len(), 2);
    let second = requests[1].body.encode();
    assert!(second.contains("Collaborators cannot delegate"));
    assert!(second.contains("owned source"));
}

#[test]
fn group_creation_starts_concurrent_children_and_preserves_one_group_receipt() {
    let project = Directory::new();
    let home = Directory::new();
    let command = || {
        tool_call(
            "check",
            "bash",
            Value::object([("command", Value::string("sleep 1"))]),
        )
    };
    let fixture = HttpFixture::new(vec![
        (200, completion("", vec![command()])),
        (200, completion("", vec![command()])),
        (200, completion("One", vec![])),
        (200, completion("Two", vec![])),
    ]);
    let mut parent = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(project.path()).unwrap(),
    );
    parent.enable_sessions(home.path()).unwrap();
    parent
        .permissions()
        .set_level(crate::permissions::Level::Full)
        .unwrap();
    let mut args = create();
    let task = args.get("tasks").and_then(Value::as_array).unwrap()[0].clone();
    if let Value::Object(fields) = &mut args {
        fields.insert("tasks".into(), Value::Array(vec![task.clone(), task]));
    }
    parent.agent_tool(&args).unwrap();
    let observer = parent.collaboration();
    let started = std::time::Instant::now();
    loop {
        let records = observer.snapshot();
        if records.len() == 2
            && records.iter().all(|record| {
                record
                    .get("tools")
                    .and_then(Value::as_array)
                    .is_some_and(|tools| !tools.is_empty())
            })
        {
            assert!(
                records
                    .iter()
                    .all(|record| record.get("state").and_then(Value::as_str) == Some("running"))
            );
            break;
        }
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "children did not overlap"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    let receipts = observer.events_since(0).1;
    assert_eq!(
        receipts[0].get("kind").and_then(Value::as_str),
        Some("created")
    );
    assert_eq!(receipts[0].get("id"), Some(&Value::Null));
    assert_eq!(
        receipts
            .iter()
            .filter(|event| event.get("kind").and_then(Value::as_str) == Some("created"))
            .count(),
        1
    );
    observer
        .wait(
            None,
            Duration::from_secs(5),
            &crate::cancel::Cancellation::default(),
        )
        .unwrap();
    observer.stop_all();
    assert_eq!(fixture.finish().len(), 4);
}
