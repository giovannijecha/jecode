use super::tests::create;
use super::*;
use crate::{
    backend::openrouter::OpenRouter,
    cancel::Cancellation,
    test_support::{Directory, HttpFixture, completion, tool_call},
};

fn wait(parent: &Agent) -> Value {
    parent
        .collaboration()
        .wait(Some("a1"), Duration::from_secs(5), &Cancellation::default())
        .unwrap()
        .remove(0)
}

#[test]
fn context_only_update_keeps_the_assignment_after_return_and_resume() {
    let project = Directory::new();
    let home = Directory::new();
    let fixture = HttpFixture::new(vec![
        (200, completion("First report", vec![])),
        (200, completion("Updated report", vec![])),
        (200, completion("Follow-up report", vec![])),
    ]);
    let mut parent = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(project.path()).unwrap(),
    );
    parent.enable_sessions(home.path()).unwrap();
    parent.agent_tool(&create()).unwrap();
    wait(&parent);
    parent
        .agent_tool(&Value::object([
            ("action", Value::string("update")),
            ("id", Value::string("a1")),
            (
                "context",
                Value::string("The data contract now includes an ID"),
            ),
        ]))
        .unwrap();
    let returned = wait(&parent);
    assert_eq!(returned.get("received_version"), Some(&Value::number(2)));
    assert_eq!(
        returned.get("instructions").and_then(Value::as_str),
        Some("Report the source state")
    );
    assert_eq!(returned.get("reviewed"), Some(&Value::Bool(false)));
    parent
        .agent_tool(&Value::object([
            ("action", Value::string("update")),
            ("id", Value::string("a1")),
            (
                "instructions",
                Value::string("Check the revised data contract"),
            ),
        ]))
        .unwrap();
    let returned = wait(&parent);
    assert_eq!(returned.get("received_version"), Some(&Value::number(3)));
    let session = parent.sessions().unwrap().id();
    parent.save_session().unwrap();
    drop(parent);

    let mut resumed = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(project.path()).unwrap(),
    );
    resumed.enable_sessions(home.path()).unwrap();
    resumed.resume(&session).unwrap();
    let restored = &resumed.collaboration().snapshot()[0];
    for field in ["instructions", "context", "received_version"] {
        assert_eq!(restored.get(field), returned.get(field));
    }

    let requests = fixture.finish();
    let updated = requests[1].body.encode();
    assert!(updated.contains("Assignment v2: Report the source state"));
    assert!(updated.contains("The data contract now includes an ID"));
    let followup = requests[2]
        .body
        .get("messages")
        .and_then(Value::as_array)
        .unwrap();
    let latest = followup
        .iter()
        .rev()
        .find(|message| message.get("role").and_then(Value::as_str) == Some("user"))
        .unwrap()
        .get("content")
        .and_then(Value::as_str)
        .unwrap();
    assert!(latest.contains("Assignment v3: Check the revised data contract"));
    assert!(latest.contains("The data contract now includes an ID"));
}

#[test]
fn partial_updates_inherit_the_latest_queued_assignment() {
    let project = Directory::new();
    let home = Directory::new();
    let fixture = HttpFixture::new(vec![(200, completion("Final report", vec![]))]);
    let mut parent = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(project.path()).unwrap(),
    );
    parent.enable_sessions(home.path()).unwrap();
    let observer = parent.collaboration();
    let gate = Arc::new((std::sync::Mutex::new(false), std::sync::Condvar::new()));
    observer
        .create(
            project.path(),
            Spec {
                title: "Inspect".into(),
                instructions: "Original assignment".into(),
                context: "Original context".into(),
                mode: Mode::Shared,
            },
            |id, root| parent.new_child(id, root, Mode::Shared),
            gate.clone(),
        )
        .unwrap();
    let results = [
        ("instructions", "Revised assignment"),
        ("context", "New contract"),
        ("instructions", "Final assignment"),
    ]
    .map(|(key, text)| {
        parent.agent_tool(&Value::object([
            ("action", Value::string("update")),
            ("id", Value::string("a1")),
            (key, Value::string(text)),
        ]))
    });
    let (lock, changed) = &*gate;
    *lock.lock().unwrap() = true;
    changed.notify_all();
    for result in results {
        result.unwrap();
    }
    assert_eq!(
        wait(&parent).get("received_version"),
        Some(&Value::number(4))
    );
    let requests = fixture.finish();
    let request = requests[0].body.encode();
    assert!(request.contains("Assignment v4: Final assignment"));
    assert!(request.contains("New contract"));
    assert!(!request.contains("Original assignment"));
    assert!(!request.contains("Original context"));
    assert!(!request.contains("Revised assignment"));
}

#[test]
fn invalid_updates_leave_the_assignment_and_receipts_unchanged() {
    let project = Directory::new();
    let home = Directory::new();
    let fixture = HttpFixture::new(vec![(200, completion("Report", vec![]))]);
    let mut parent = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(project.path()).unwrap(),
    );
    parent.enable_sessions(home.path()).unwrap();
    parent.agent_tool(&create()).unwrap();
    wait(&parent);
    let before = parent.collaboration().snapshot();
    let receipts = parent.collaboration().events_since(0).1;
    for (fields, error) in [
        (vec![], "update requires instructions or context"),
        (
            vec![("instructions", Value::number(1))],
            "instructions must be a string",
        ),
        (vec![("context", Value::Null)], "context must be a string"),
    ] {
        let mut arguments = Value::object([
            ("action", Value::string("update")),
            ("id", Value::string("a1")),
        ]);
        let Value::Object(arguments_fields) = &mut arguments else {
            unreachable!()
        };
        arguments_fields.extend(fields.into_iter().map(|(key, value)| (key.into(), value)));
        assert_eq!(parent.agent_tool(&arguments).unwrap_err(), error);
        assert_eq!(parent.collaboration().snapshot(), before);
        assert_eq!(parent.collaboration().events_since(0).1, receipts);
    }
    fixture.finish();
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
    parent
        .agent_tool(&Value::object([
            ("action", Value::string("update")),
            ("id", Value::string("a1")),
            (
                "context",
                Value::string("The data contract now includes an ID"),
            ),
        ]))
        .unwrap();
    let state = observer.snapshot();
    assert_eq!(
        state[0].get("sent_version").and_then(Value::as_usize),
        Some(3)
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
        Some(3)
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
    assert!(
        requests[1]
            .body
            .encode()
            .contains("The data contract now includes an ID")
    );
}
