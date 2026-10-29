use super::*;
use crate::{
    backend::{ToolCall, openrouter::OpenRouter},
    collaboration::{Spec, workspace::Mode},
    events::Event,
    test_support::{Directory, HttpFixture, completion, tool_call},
};
use std::{fs, process::Command};

#[test]
fn failed_group_receipt_cancels_children_before_start_gate_opens() {
    let project = Directory::new();
    let home = Directory::new();
    let server = crate::test_support::server::Server::start(|listener, stop| {
        usize::from(crate::test_support::server::accept(&listener, &stop).is_ok())
    });
    let endpoint = format!("{}/chat/completions", server.endpoint);
    let mut parent = Agent::new(
        OpenRouter::fixture(endpoint),
        Tools::new(project.path()).unwrap(),
    );
    parent.enable_sessions(home.path()).unwrap();
    let handle = parent.sessions().unwrap();
    handle
        .delete(&std::collections::BTreeSet::new(), || {})
        .unwrap();

    let error = parent.agent_tool(&create("shared")).unwrap_err();
    assert!(error.contains("session was deleted"));
    assert!(parent.collaboration().snapshot().is_empty());
    assert!(parent.collaboration().events_since(0).1.is_empty());
    assert_eq!(
        server.finish(),
        0,
        "cancelled child must not reach the backend"
    );
}

#[test]
fn update_before_worker_handoff_is_received_once_as_initial_assignment() {
    let project = Directory::new();
    let home = Directory::new();
    let fixture = HttpFixture::new(vec![(200, completion("Version two report", vec![]))]);
    let mut parent = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(project.path()).unwrap(),
    );
    parent.enable_sessions(home.path()).unwrap();
    let observer = parent.collaboration();
    let gate = std::sync::Arc::new((std::sync::Mutex::new(false), std::sync::Condvar::new()));
    observer
        .create(
            project.path(),
            Spec {
                title: "Inspect".into(),
                instructions: "Version one".into(),
                context: String::new(),
                mode: Mode::Shared,
            },
            |id, root| parent.new_child(id, root, Mode::Shared),
            gate.clone(),
        )
        .unwrap();
    observer
        .receipt("created", None, 0, "Agents", vec![])
        .unwrap();
    observer
        .update("a1", "Version two".into(), String::new())
        .unwrap();
    let (lock, changed) = &*gate;
    *lock.lock().unwrap() = true;
    changed.notify_all();
    let records = observer
        .wait(
            Some("a1"),
            Duration::from_secs(5),
            &crate::cancel::Cancellation::default(),
        )
        .unwrap();
    assert_eq!(
        records[0].get("received_version").and_then(Value::as_usize),
        Some(2)
    );
    let receipts = observer.events_since(0).1;
    assert_eq!(
        receipts
            .iter()
            .filter(|event| event.get("kind").and_then(Value::as_str) == Some("received"))
            .count(),
        1
    );
    let requests = fixture.finish();
    assert_eq!(requests.len(), 1);
    assert!(requests[0].body.encode().contains("Version two"));
    assert!(!requests[0].body.encode().contains("Version one"));
}

pub(super) fn create(mode: &str) -> Value {
    Value::object([
        ("action", Value::string("create")),
        (
            "tasks",
            Value::Array(vec![Value::object([
                ("title", Value::string("Inspect")),
                ("instructions", Value::string("Report")),
                ("workspace", Value::string(mode)),
            ])]),
        ),
    ])
}

pub(super) fn action(name: &str) -> Value {
    Value::object([("action", Value::string(name)), ("id", Value::string("a1"))])
}

pub(super) fn repository() -> (Directory, std::path::PathBuf) {
    let temp = Directory::new();
    let project = temp.path().join("project");
    fs::create_dir(&project).unwrap();
    let git = |args: &[&str]| {
        let output = Command::new("git")
            .arg("-C")
            .arg(&project)
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    };
    git(&["init", "-q"]);
    fs::write(project.join("file.txt"), "source\n").unwrap();
    git(&["add", "file.txt"]);
    git(&[
        "-c",
        "user.name=Giovanni Jecha",
        "-c",
        "user.email=100841799+giovannijecha@users.noreply.github.com",
        "commit",
        "-qm",
        "initial",
    ]);
    (temp, project)
}

#[test]
fn closed_isolated_workspace_leaves_history_but_not_active_context_after_resume() {
    let (_temp, project) = repository();
    let home = Directory::new();
    let fixture = HttpFixture::new(vec![
        (200, completion("Completed report", vec![])),
        (200, completion("Next report", vec![])),
    ]);
    let mut parent = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(&project).unwrap(),
    );
    parent.enable_sessions(home.path()).unwrap();
    parent.agent_tool(&create("isolated")).unwrap();
    parent
        .collaboration()
        .wait(
            Some("a1"),
            Duration::from_secs(5),
            &crate::cancel::Cancellation::default(),
        )
        .unwrap();
    parent.agent_tool(&action("close")).unwrap();
    assert!(parent.collaboration().snapshot().is_empty());
    assert!(!parent.request_environment().contains("a1 Inspect"));
    let close_event = parent
        .collaboration()
        .events_since(0)
        .1
        .into_iter()
        .find(|event| event.get("kind").and_then(Value::as_str) == Some("closed"))
        .unwrap();
    assert_eq!(close_event.get("version"), Some(&Value::number(1)));
    let closed = &close_event
        .get("records")
        .and_then(Value::as_array)
        .unwrap()[0];
    assert_eq!(closed.get("workspace_closed"), Some(&Value::Bool(true)));
    assert_eq!(
        closed
            .get("report")
            .and_then(|report| report.get("text"))
            .and_then(Value::as_str),
        Some("Completed report")
    );
    let id = parent.sessions().unwrap().id();
    parent.save_session().unwrap();
    drop(parent);

    let mut resumed = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(&project).unwrap(),
    );
    resumed.enable_sessions(home.path()).unwrap();
    resumed.resume(&id).unwrap();
    assert!(resumed.collaboration().snapshot().is_empty());
    assert!(!resumed.request_environment().contains("a1 Inspect"));
    let created = resumed.agent_tool(&create("isolated")).unwrap();
    assert_eq!(
        created
            .get("created")
            .and_then(Value::as_array)
            .and_then(|items| items.first())
            .and_then(|item| item.get("id"))
            .and_then(Value::as_str),
        Some("a2")
    );
    resumed
        .collaboration()
        .wait(
            Some("a2"),
            Duration::from_secs(5),
            &crate::cancel::Cancellation::default(),
        )
        .unwrap();
    assert_eq!(resumed.collaboration().snapshot().len(), 1);
    resumed
        .agent_tool(&Value::object([
            ("action", Value::string("close")),
            ("id", Value::string("a2")),
        ]))
        .unwrap();
    assert!(resumed.collaboration().snapshot().is_empty());
    assert_eq!(fixture.finish().len(), 2);
}

#[test]
fn closing_idle_child_stops_its_persistent_process() {
    let project = Directory::new();
    let home = Directory::new();
    let fixture = HttpFixture::new(vec![
        (
            200,
            completion(
                "",
                vec![tool_call(
                    "heartbeat",
                    "bash",
                    Value::object([
                        (
                            "command",
                            Value::string("while :; do printf x >> heartbeat; sleep 0.05; done"),
                        ),
                        ("wait_ms", Value::number(0)),
                    ]),
                )],
            ),
        ),
        (200, completion("Heartbeat started", vec![])),
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
    parent.agent_tool(&create("shared")).unwrap();
    parent
        .collaboration()
        .wait(
            Some("a1"),
            Duration::from_secs(5),
            &crate::cancel::Cancellation::default(),
        )
        .unwrap();
    let heartbeat = project.path().join("heartbeat");
    let started = std::time::Instant::now();
    while std::fs::metadata(&heartbeat).is_err() {
        assert!(started.elapsed() < Duration::from_secs(5));
        std::thread::sleep(Duration::from_millis(20));
    }
    let running_size = std::fs::metadata(&heartbeat).unwrap().len();
    std::thread::sleep(Duration::from_millis(150));
    assert!(std::fs::metadata(&heartbeat).unwrap().len() > running_size);
    parent.agent_tool(&action("close")).unwrap();
    let stopped_size = std::fs::metadata(&heartbeat).unwrap().len();
    std::thread::sleep(Duration::from_millis(400));
    assert_eq!(std::fs::metadata(&heartbeat).unwrap().len(), stopped_size);
    assert!(parent.collaboration().snapshot().is_empty());
    assert_eq!(fixture.finish().len(), 2);
}

#[test]
fn recovered_return_without_worker_rejects_mutations_without_changing_report() {
    let project = Directory::new();
    let home = Directory::new();
    let fixture = HttpFixture::new(vec![(200, completion("Completed report", vec![]))]);
    let mut parent = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(project.path()).unwrap(),
    );
    parent.enable_sessions(home.path()).unwrap();
    parent.agent_tool(&create("shared")).unwrap();
    parent
        .collaboration()
        .wait(
            Some("a1"),
            Duration::from_secs(5),
            &crate::cancel::Cancellation::default(),
        )
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
    let before = resumed.collaboration().snapshot();
    assert_eq!(
        before[0].get("state").and_then(Value::as_str),
        Some("returned")
    );
    let update = Value::object([
        ("action", Value::string("update")),
        ("id", Value::string("a1")),
        ("instructions", Value::string("Try again")),
    ]);
    assert!(resumed.agent_tool(&update).is_err());
    assert!(resumed.agent_tool(&action("stop")).is_err());
    assert_eq!(resumed.collaboration().snapshot(), before);
    assert_eq!(fixture.finish().len(), 1);
}

#[test]
fn missing_isolated_workspace_marks_only_that_record_unknown_on_resume() {
    let (_temp, project) = repository();
    let home = Directory::new();
    let fixture = HttpFixture::new(vec![(200, completion("Completed report", vec![]))]);
    let mut parent = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(&project).unwrap(),
    );
    parent.enable_sessions(home.path()).unwrap();
    parent.agent_tool(&create("isolated")).unwrap();
    parent
        .collaboration()
        .wait(
            Some("a1"),
            Duration::from_secs(5),
            &crate::cancel::Cancellation::default(),
        )
        .unwrap();
    let record = &parent.collaboration().snapshot()[0];
    let marker = std::path::Path::new(
        record
            .get("workspace_info")
            .unwrap()
            .get("storage")
            .unwrap()
            .as_str()
            .unwrap(),
    )
    .join("baseline.json");
    let id = parent.sessions().unwrap().id();
    parent.save_session().unwrap();
    drop(parent);
    fs::remove_file(marker).unwrap();

    let mut resumed = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(&project).unwrap(),
    );
    resumed.enable_sessions(home.path()).unwrap();
    resumed.resume(&id).unwrap();
    let record = &resumed.collaboration().snapshot()[0];
    assert_eq!(record.get("state").and_then(Value::as_str), Some("unknown"));
    assert!(
        record
            .get("error")
            .and_then(Value::as_str)
            .unwrap()
            .starts_with("Workspace unavailable:")
    );
    assert_eq!(
        record
            .get("report")
            .and_then(|v| v.get("text"))
            .and_then(Value::as_str),
        Some("Completed report")
    );
    assert_eq!(fixture.finish().len(), 1);
}

#[test]
fn malformed_parallel_agent_call_has_no_side_effect() {
    let project = Directory::new();
    let home = Directory::new();
    let mut parent = Agent::new(
        OpenRouter::fixture("http://127.0.0.1:1".into()),
        Tools::new(project.path()).unwrap(),
    );
    parent.enable_sessions(home.path()).unwrap();
    let call = ToolCall {
        id: "call-1".into(),
        name: "agent".into(),
        arguments: r#"{"action":"create","parallel":"true","tasks":[{"title":"Inspect","instructions":"Report","workspace":"shared"}]}"#.into(),
    };
    let result = parent.execute_tool(&call);
    assert_eq!(
        result.get("error").and_then(Value::as_str),
        Some("parallel must be a boolean")
    );
    assert!(parent.collaboration().snapshot().is_empty());
}

#[test]
fn malformed_optional_parameters_fail_instead_of_using_defaults() {
    let project = Directory::new();
    let home = Directory::new();
    let mut parent = Agent::new(
        OpenRouter::fixture("http://127.0.0.1:1".into()),
        Tools::new(project.path()).unwrap(),
    );
    parent.enable_sessions(home.path()).unwrap();
    let wait = Value::object([
        ("action", Value::string("wait")),
        ("timeout_ms", Value::string("0")),
    ]);
    assert_eq!(
        parent.agent_tool(&wait).unwrap_err(),
        "timeout_ms must be an integer"
    );
    let create = Value::object([
        ("action", Value::string("create")),
        (
            "tasks",
            Value::Array(vec![Value::object([
                ("title", Value::string("Inspect")),
                ("instructions", Value::string("Report")),
                ("workspace", Value::string("shared")),
                ("context", Value::number(1)),
            ])]),
        ),
    ]);
    assert_eq!(
        parent.agent_tool(&create).unwrap_err(),
        "context must be a string"
    );
    assert!(parent.collaboration().snapshot().is_empty());
    let integrate = Value::object([
        ("action", Value::string("integrate")),
        ("id", Value::string("a1")),
        ("paths", Value::Array(vec![])),
        ("expected_review_token", Value::string("token")),
    ]);
    assert_eq!(
        parent.agent_tool(&integrate).unwrap_err(),
        "paths must contain at least one file"
    );
}

#[test]
fn final_collaboration_flush_propagates_sink_error_and_checkpoints_turn() {
    let project = Directory::new();
    let home = Directory::new();
    let fixture = HttpFixture::new(vec![(200, completion("Main answer", vec![]))]);
    let mut parent = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(project.path()).unwrap(),
    );
    parent.enable_sessions(home.path()).unwrap();
    let observer = parent.collaboration();
    let mut emitted = false;
    let result = parent.run_turn("Proceed", &mut |event| {
        match event {
            Event::Message { .. } if !emitted => {
                observer.receipt("created", None, 0, "Agents", vec![])?;
                emitted = true;
            }
            Event::Collaboration { .. } => return Err("sink failed".into()),
            _ => {}
        }
        Ok(())
    });
    assert_eq!(result.unwrap_err(), "sink failed");
    assert!(parent.archive().events.lock().unwrap().iter().any(|event| {
        event.get("type").and_then(Value::as_str) == Some("turn_error")
            && event.get("error").and_then(Value::as_str) == Some("sink failed")
    }));
    assert_eq!(fixture.finish().len(), 1);
}
