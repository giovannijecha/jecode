use super::tests::create;
use super::*;
use crate::{
    backend::openrouter::OpenRouter,
    test_support::{Directory, HttpFixture, Response, completion, tool_call},
};

struct RestoreJournalPermissions {
    path: std::path::PathBuf,
    original: std::fs::Permissions,
}

impl Drop for RestoreJournalPermissions {
    fn drop(&mut self) {
        let _ = std::fs::set_permissions(&self.path, self.original.clone());
    }
}

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
    // This bounds fixture completion, not the latency of journal writes.
    let completion_timeout = Duration::from_secs(60);
    let project = Directory::new();
    let home = Directory::new();
    std::fs::write(project.path().join("source.txt"), "source").unwrap();
    let (release, gate) = std::sync::mpsc::channel();
    let final_response = Value::object([(
        "choices",
        Value::Array(vec![Value::object([
            ("index", Value::number(0)),
            (
                "delta",
                Value::object([("content", Value::string("Completed report"))]),
            ),
            ("finish_reason", Value::string("stop")),
        ])]),
    )]);
    let fixture = HttpFixture::streaming_with_wait(
        vec![
            Response::Json(
                200,
                completion(
                    "",
                    vec![tool_call(
                        "check",
                        "read",
                        Value::object([("path", Value::string("source.txt"))]),
                    )],
                ),
            ),
            Response::GatedStream {
                head: String::new(),
                tail: format!("data: {}\n\ndata: [DONE]\n\n", final_response.encode()),
                release: gate,
            },
        ],
        completion_timeout,
    );
    let mut parent = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(project.path()).unwrap(),
    );
    parent.enable_sessions(home.path()).unwrap();
    parent.agent_tool(&create()).unwrap();
    let observer = parent.collaboration();
    let started = std::time::Instant::now();
    loop {
        let snapshot = observer.snapshot();
        let tool_done = snapshot[0]
            .get("tools")
            .and_then(Value::as_array)
            .and_then(|tools| tools.first())
            .and_then(|tool| tool.get("state"))
            .and_then(Value::as_str)
            == Some("done");
        if tool_done {
            assert_eq!(snapshot[0].get("received_version"), Some(&Value::number(1)));
            break;
        }
        assert!(
            started.elapsed() < completion_timeout,
            "collaborator did not finish its read: {snapshot:?}"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    let handle = parent.sessions().unwrap();
    let journal = handle.store().fixture_journal(&handle.id());
    let original_permissions = std::fs::metadata(&journal).unwrap().permissions();
    let restoration = RestoreJournalPermissions {
        path: journal.clone(),
        original: original_permissions.clone(),
    };
    let mut permissions = original_permissions;
    permissions.set_readonly(true);
    std::fs::set_permissions(&journal, permissions).unwrap();
    release.send(()).unwrap();
    let started = std::time::Instant::now();
    loop {
        let snapshot = observer.snapshot();
        if snapshot[0].get("state").and_then(Value::as_str) == Some("unknown") {
            break;
        }
        assert!(
            started.elapsed() < completion_timeout,
            "collaborator did not record the failed receipt: {snapshot:?}"
        );
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
    assert!(
        !observer
            .events_since(0)
            .1
            .iter()
            .any(|event| { event.get("kind").and_then(Value::as_str) == Some("returned") })
    );
    drop(restoration);
    assert!(
        !std::fs::metadata(&journal)
            .unwrap()
            .permissions()
            .readonly()
    );
    observer.stop_all();
    assert_eq!(fixture.finish().len(), 2);
}

#[test]
fn a_create_call_past_eight_assignments_creates_and_runs_them_all() {
    let project = Directory::new();
    let home = Directory::new();
    let total = 12;
    let fixture = HttpFixture::new(
        (0..total)
            .map(|_| (200, completion("Report", vec![])))
            .collect(),
    );
    let mut parent = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(project.path()).unwrap(),
    );
    parent.enable_sessions(home.path()).unwrap();
    let tasks = (0..total)
        .map(|index| {
            Value::object([
                ("title", Value::string(format!("Inspect part {index}"))),
                ("instructions", Value::string("Report the source state")),
                ("workspace", Value::string("shared")),
            ])
        })
        .collect();
    let result = parent
        .agent_tool(&Value::object([
            ("action", Value::string("create")),
            ("tasks", Value::Array(tasks)),
        ]))
        .unwrap();
    assert!(result.get("error").is_none(), "{}", result.encode());
    let created = result.get("created").and_then(Value::as_array).unwrap();
    assert_eq!(created.len(), total);
    let observer = parent.collaboration();
    let started = std::time::Instant::now();
    loop {
        let records = observer.snapshot();
        let returned = records
            .iter()
            .filter(|record| record.get("state").and_then(Value::as_str) == Some("returned"))
            .count();
        if returned == total {
            break;
        }
        assert!(
            started.elapsed() < Duration::from_secs(30),
            "{returned} of {total} returned"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(fixture.finish().len(), total);
}
#[test]
fn queued_stop_and_update_preserve_handoff_and_eight_active_workers() {
    let project = Directory::new();
    let home = Directory::new();
    let (release, gate) = std::sync::mpsc::channel();
    let streamed = Value::object([(
        "choices",
        Value::Array(vec![Value::object([
            ("index", Value::number(0)),
            (
                "delta",
                Value::object([("content", Value::string("Report"))]),
            ),
            ("finish_reason", Value::string("stop")),
        ])]),
    )]);
    let mut responses = vec![Response::GatedStream {
        head: String::new(),
        tail: format!("data: {}\n\ndata: [DONE]\n\n", streamed.encode()),
        release: gate,
    }];
    responses.extend((0..8).map(|_| Response::Json(200, completion("Report", vec![]))));
    let fixture = HttpFixture::streaming_with_wait(responses, Duration::from_secs(20));
    let mut parent = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(project.path()).unwrap(),
    );
    parent.enable_sessions(home.path()).unwrap();
    let tasks = (1..=10)
        .map(|index| {
            Value::object([
                ("title", Value::string(format!("Inspect {index}"))),
                ("instructions", Value::string("Original instruction")),
                ("workspace", Value::string("shared")),
            ])
        })
        .collect();
    parent
        .agent_tool(&Value::object([
            ("action", Value::string("create")),
            ("tasks", Value::Array(tasks)),
        ]))
        .unwrap();
    let observer = parent.collaboration();
    let started = std::time::Instant::now();
    loop {
        let (running, queue, busy) = observer.slot_stats();
        if running == 8 && queue.len() == 2 {
            assert_eq!(queue, ["a9", "a10"]);
            assert_eq!(busy, 8);
            break;
        }
        assert!(started.elapsed() < Duration::from_secs(10));
        std::thread::sleep(Duration::from_millis(5));
    }
    parent
        .agent_tool(&Value::object([
            ("action", Value::string("stop")),
            ("id", Value::string("a10")),
        ]))
        .unwrap();
    let stopped = observer
        .snapshot()
        .into_iter()
        .find(|record| record.get("id").and_then(Value::as_str) == Some("a10"))
        .unwrap();
    assert_eq!(
        stopped.get("state").and_then(Value::as_str),
        Some("stopped")
    );
    assert_eq!(stopped.get("received_version"), Some(&Value::number(0)));
    assert!(!observer.events_since(0).1.iter().any(|event| {
        event.get("id").and_then(Value::as_str) == Some("a10")
            && event.get("kind").and_then(Value::as_str) == Some("received")
    }));
    parent
        .agent_tool(&Value::object([
            ("action", Value::string("update")),
            ("id", Value::string("a9")),
            ("instructions", Value::string("Latest instruction")),
        ]))
        .unwrap();
    release.send(()).unwrap();
    let records = observer
        .wait(
            Some("a9"),
            Duration::from_secs(20),
            &crate::cancel::Cancellation::default(),
        )
        .unwrap();
    assert_eq!(
        records[0].get("state").and_then(Value::as_str),
        Some("returned")
    );
    assert_eq!(records[0].get("received_version"), Some(&Value::number(2)));
    let requests = fixture.finish();
    assert_eq!(requests.len(), 9);
    let updated = requests
        .iter()
        .find(|request| request.body.encode().contains("Latest instruction"))
        .unwrap()
        .body
        .encode();
    assert!(updated.contains("Assignment v2: Latest instruction"));
    assert!(!updated.contains("Original instruction"));
}
