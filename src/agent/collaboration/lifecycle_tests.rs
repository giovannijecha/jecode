use super::tests::create;
use super::*;
use crate::{
    backend::openrouter::OpenRouter,
    test_support::{Directory, HttpFixture, completion, tool_call},
};

#[test]
fn stop_active_child_and_cancel_parent_wait_promptly() {
    let project = Directory::new();
    let home = Directory::new();
    let fixture = HttpFixture::new(vec![(
        200,
        completion(
            "",
            vec![tool_call(
                "long",
                "bash",
                Value::object([("command", Value::string("sleep 3"))]),
            )],
        ),
    )]);
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
        assert!(started.elapsed() < Duration::from_secs(5));
        std::thread::sleep(Duration::from_millis(10));
    }
    let cancellation = crate::cancel::Cancellation::default();
    let token = cancellation.clone();
    let waiting = observer.clone();
    let task =
        std::thread::spawn(move || waiting.wait(Some("a1"), Duration::from_secs(60), &token));
    std::thread::sleep(Duration::from_millis(50));
    cancellation.cancel();
    assert_eq!(task.join().unwrap().unwrap_err(), "Operation cancelled");
    parent
        .agent_tool(&Value::object([
            ("action", Value::string("stop")),
            ("id", Value::string("a1")),
        ]))
        .unwrap();
    let stopped = observer
        .wait(
            Some("a1"),
            Duration::from_secs(5),
            &crate::cancel::Cancellation::default(),
        )
        .unwrap();
    assert_eq!(
        stopped[0].get("state").and_then(Value::as_str),
        Some("stopped")
    );
    observer.close("a1").unwrap();
    assert_eq!(fixture.finish().len(), 1);
}

#[test]
fn observer_and_journal_mask_credentials_in_assignments_and_reports() {
    let project = Directory::new();
    let home = Directory::new();
    let fixture = HttpFixture::new(vec![
        (200, completion("Seed", vec![])),
        (200, completion("private-key in report", vec![])),
    ]);
    let mut parent = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(project.path()).unwrap(),
    );
    parent.include_credentials(crate::redact::Redactor::new("private-key".into()));
    parent.enable_sessions(home.path()).unwrap();
    parent.run_turn("Seed session", &mut |_| Ok(())).unwrap();
    let mut args = create();
    if let Value::Object(fields) = &mut args
        && let Some(Value::Array(tasks)) = fields.get_mut("tasks")
        && let Value::Object(task) = &mut tasks[0]
    {
        task.insert(
            "instructions".into(),
            Value::string("Inspect private-key source"),
        );
    }
    parent.agent_tool(&args).unwrap();
    let observer = parent.collaboration();
    observer
        .wait(
            Some("a1"),
            Duration::from_secs(5),
            &crate::cancel::Cancellation::default(),
        )
        .unwrap();
    let snapshot = Value::Array(observer.snapshot()).encode();
    assert!(!snapshot.contains("private-key"));
    assert!(snapshot.contains("[redacted]"));
    let events = Value::Array(parent.sessions().unwrap().snapshot().events).encode();
    assert!(!events.contains("private-key"));
    observer.stop_all();
    assert_eq!(fixture.finish().len(), 2);
}

#[test]
fn failed_parent_receipt_never_claims_a_confirmed_child_return() {
    let project = Directory::new();
    let home = Directory::new();
    let fixture = HttpFixture::new(vec![
        (
            200,
            completion(
                "",
                vec![tool_call(
                    "check",
                    "bash",
                    Value::object([("command", Value::string("sleep 0.4"))]),
                )],
            ),
        ),
        (200, completion("Completed report", vec![])),
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
        assert!(started.elapsed() < Duration::from_secs(5));
        std::thread::sleep(Duration::from_millis(10));
    }
    let handle = parent.sessions().unwrap();
    let journal = handle
        .store()
        .bucket()
        .join(format!("{}.jsonl", handle.id()));
    let original_permissions = std::fs::metadata(&journal).unwrap().permissions();
    let mut permissions = original_permissions.clone();
    permissions.set_readonly(true);
    std::fs::set_permissions(&journal, permissions).unwrap();
    let started = std::time::Instant::now();
    while observer.snapshot()[0].get("state").and_then(Value::as_str) != Some("unknown") {
        assert!(started.elapsed() < Duration::from_secs(5));
        std::thread::sleep(Duration::from_millis(10));
    }
    let record = &observer.snapshot()[0];
    assert!(
        record
            .get("error")
            .and_then(Value::as_str)
            .unwrap()
            .contains("Could not save collaborator receipt")
    );
    std::fs::set_permissions(&journal, original_permissions).unwrap();
    observer.stop_all();
    assert_eq!(fixture.finish().len(), 2);
}
