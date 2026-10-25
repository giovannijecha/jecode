use super::*;
use crate::test_support::{Directory, HttpFixture, completion, tool_call};
use std::time::{Duration, Instant};

const HEARTBEAT: &str = "(while :; do printf x >> heartbeat; sleep 0.05; done) & printf 'ready\\n'";

fn agent(home: &Directory, directory: &Directory, endpoint: &str) -> Agent {
    let mut agent = Agent::new(
        OpenRouter::fixture(endpoint.into()),
        Tools::new(directory.path()).unwrap(),
    );
    agent.enable_sessions(home.path()).unwrap();
    agent
}

fn heartbeat(directory: &Directory) -> u64 {
    std::fs::metadata(directory.path().join("heartbeat")).map_or(0, |metadata| metadata.len())
}

fn alive(directory: &Directory) -> bool {
    let started = Instant::now();
    let before = heartbeat(directory);
    while started.elapsed() < Duration::from_secs(5) {
        std::thread::sleep(Duration::from_millis(50));
        if heartbeat(directory) > before {
            return true;
        }
    }
    false
}

fn stopped(directory: &Directory) -> bool {
    let before = heartbeat(directory);
    std::thread::sleep(Duration::from_millis(400));
    heartbeat(directory) == before
}

fn poll(agent: &Agent, id: &str) -> Value {
    agent.tools.execute(
        "process",
        &Value::object([
            ("action", Value::string("poll")),
            ("process_id", Value::string(id)),
        ])
        .encode(),
    )
}

fn start(agent: &Agent) -> String {
    let result = agent.tools.execute(
        "bash",
        &Value::object([
            ("command", Value::string(HEARTBEAT)),
            ("wait_ms", Value::number(0)),
        ])
        .encode(),
    );
    result
        .get("process_id")
        .and_then(Value::as_str)
        .unwrap()
        .into()
}

#[test]
fn persistent_processes_span_turns_and_end_only_with_their_conversation() {
    let home = Directory::new();
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![
        (
            200,
            completion(
                "Starting",
                vec![tool_call(
                    "start",
                    "bash",
                    Value::object([
                        ("command", Value::string(HEARTBEAT)),
                        ("wait_ms", Value::number(0)),
                    ]),
                )],
            ),
        ),
        (200, completion("Started", vec![])),
        (
            200,
            completion(
                "Checking",
                vec![tool_call(
                    "check",
                    "process",
                    Value::object([("action", Value::string("list"))]),
                )],
            ),
        ),
        (200, completion("Still running", vec![])),
    ]);
    let mut agent = agent(&home, &directory, &fixture.endpoint);
    agent
        .run_turn("Start the watcher", &mut |event| match event {
            Event::Recovering { error, .. } => Err(format!("Unexpected fixture recovery: {error}")),
            _ => Ok(()),
        })
        .unwrap();
    let result = agent
        .messages
        .lock()
        .unwrap()
        .iter()
        .find(|message| message.get("tool_call_id").and_then(Value::as_str) == Some("start"))
        .and_then(|message| message.get("content").and_then(Value::as_str))
        .map(|content| crate::json::parse(content).unwrap())
        .unwrap();
    assert_eq!(
        result.get("status").and_then(Value::as_str),
        Some("running")
    );
    let id = result
        .get("process_id")
        .and_then(Value::as_str)
        .unwrap()
        .to_string();
    assert!(alive(&directory));

    // Interrupting a turn does not own the process; a later turn still sees it.
    agent.cancellation().cancel();
    agent.cancellation().reset();
    agent
        .run_turn("Check the watcher", &mut |event| match event {
            Event::Recovering { error, .. } => Err(format!("Unexpected fixture recovery: {error}")),
            _ => Ok(()),
        })
        .unwrap();
    let listed = agent.messages.lock().unwrap().iter().any(|message| {
        message.get("tool_call_id").and_then(Value::as_str) == Some("check")
            && message
                .get("content")
                .and_then(Value::as_str)
                .is_some_and(|content| content.contains(&id) && content.contains("running"))
    });
    assert!(listed);
    assert!(alive(&directory));

    // A failed switch keeps the current conversation's processes.
    assert!(agent.resume("1-2-3").is_err());
    assert!(alive(&directory));
    let original = agent.sessions().unwrap().id();

    agent
        .start_new(agent.model().into(), agent.effort())
        .unwrap();
    assert!(stopped(&directory));
    assert!(poll(&agent, &id).get("error").is_some());

    // A resumed conversation shows historical IDs without restarting anything.
    let second = start(&agent);
    assert!(alive(&directory));
    agent.resume(&original).unwrap();
    assert!(stopped(&directory));
    for id in [&id, &second] {
        let stale = poll(&agent, id);
        assert!(
            stale
                .get("error")
                .and_then(Value::as_str)
                .is_some_and(|error| error.contains("never restarted")),
            "{stale:?}"
        );
    }
    assert!(stopped(&directory));
    assert_eq!(fixture.finish().len(), 4);
}

#[test]
fn deleting_the_live_session_stops_its_processes() {
    let home = Directory::new();
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![]);
    let mut agent = agent(&home, &directory, &fixture.endpoint);
    let id = start(&agent);
    assert!(alive(&directory));
    let current = agent.sessions().unwrap().id();
    let (_, replaced) = agent
        .delete_session(&current, agent.model().into(), agent.effort())
        .unwrap();
    assert!(replaced);
    assert!(stopped(&directory));
    assert!(poll(&agent, &id).get("error").is_some());
}

#[test]
fn failed_live_session_deletion_preserves_its_processes() {
    let home = Directory::new();
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![]);
    let mut agent = agent(&home, &directory, &fixture.endpoint);
    let process_id = start(&agent);
    assert!(alive(&directory));
    let session = agent.sessions().unwrap();
    let session_id = session.id();
    // A directory where a session sidecar belongs blocks deletion.
    let blocker = session
        .store()
        .bucket()
        .join(format!("{session_id}.tmp-1-2-3"));
    std::fs::create_dir(&blocker).unwrap();

    let error = agent
        .delete_session(&session_id, agent.model().into(), agent.effort())
        .unwrap_err();
    assert!(error.contains("special entry"), "{error}");
    assert_eq!(agent.sessions().unwrap().id(), session_id);
    assert_eq!(
        poll(&agent, &process_id)
            .get("status")
            .and_then(Value::as_str),
        Some("running")
    );
    assert!(alive(&directory));

    std::fs::remove_dir(blocker).unwrap();
}
