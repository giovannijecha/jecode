use super::*;
use crate::json::Value;
use crate::openrouter::OpenRouter;
use crate::test_support::{Directory, HttpFixture, completion, tool_call};
use crate::tools::Tools;
use std::time::{Duration, Instant};

fn turn(agent: &mut Agent, fixture: &HttpFixture, prompt: &str) {
    agent.replace_client(OpenRouter::fixture(fixture.endpoint.clone()));
    agent
        .run_turn(prompt.to_string(), &mut |_| Ok::<(), String>(()))
        .unwrap();
}

fn printed(agent: &Agent, arguments: &str) -> String {
    let mut output = Vec::new();
    print(agent, arguments, &mut output).unwrap();
    String::from_utf8(output).unwrap()
}

#[test]
fn plain_terminal_prints_locally_without_consuming_model_output() {
    let directory = Directory::new();
    let launch = HttpFixture::new(vec![
        (
            200,
            completion(
                "",
                vec![tool_call(
                    "bash-1",
                    "bash",
                    Value::object([
                        (
                            "command",
                            Value::string(
                                "until [ -e go ]; do sleep 0.02; done; printf 'plain-out\\n'; printf '\\033[2Jplain-err\\n' >&2; exit 4",
                            ),
                        ),
                        ("wait_ms", Value::number(0)),
                    ]),
                )],
            ),
        ),
        (200, completion("Started.", vec![])),
    ]);
    let mut agent = Agent::new(
        OpenRouter::fixture(launch.endpoint.clone()),
        Tools::new(directory.path()).unwrap(),
    );
    assert_eq!(
        printed(&agent, ""),
        "No active background processes in this conversation.\n"
    );
    turn(&mut agent, &launch, "Start");
    let observer = agent.processes();
    let id = observer.processes().unwrap()[0].id.clone();
    assert!(printed(&agent, "").contains(&id));
    std::fs::write(directory.path().join("go"), "").unwrap();
    let started = Instant::now();
    while observer.processes().unwrap()[0].active() {
        assert!(started.elapsed() < Duration::from_secs(10));
        std::thread::sleep(Duration::from_millis(20));
    }
    let messages = agent.archive().messages.lock().unwrap().len();
    let events = agent.archive().events.lock().unwrap().len();

    assert_eq!(
        printed(&agent, ""),
        "No active background processes in this conversation.\n"
    );
    let details = printed(&agent, &id);
    assert!(
        details.starts_with(&format!("{id} · exited 4\n")),
        "{details}"
    );
    assert!(details.contains("Output:\nplain-out\n"), "{details}");
    // Saved control sequences cannot steer the console.
    assert!(
        details.contains("Errors:\n\u{fffd}[2Jplain-err\n"),
        "{details}"
    );
    assert_eq!(printed(&agent, &id), details);
    let mut output = Vec::new();
    let error = print(&agent, "process-1-2-3", &mut output).unwrap_err();
    assert!(error.contains("No process process-1-2-3"), "{error}");
    // Local snapshots touch neither the model context nor the local history.
    assert_eq!(agent.archive().messages.lock().unwrap().len(), messages);
    assert_eq!(agent.archive().events.lock().unwrap().len(), events);
    assert_eq!(launch.finish().len(), 2);

    let poll = HttpFixture::new(vec![
        (
            200,
            completion(
                "",
                vec![tool_call(
                    "poll-1",
                    "process",
                    Value::object([
                        ("action", Value::string("poll")),
                        ("process_id", Value::string(&id)),
                    ]),
                )],
            ),
        ),
        (200, completion("Polled.", vec![])),
    ]);
    turn(&mut agent, &poll, "Poll");
    let result = agent
        .archive()
        .messages
        .lock()
        .unwrap()
        .iter()
        .find(|message| message.get("tool_call_id").and_then(Value::as_str) == Some("poll-1"))
        .and_then(|message| message.get("content").and_then(Value::as_str))
        .map(|content| crate::json::parse(content).unwrap())
        .unwrap();
    assert_eq!(
        result.get("stdout").and_then(Value::as_str),
        Some("plain-out\n")
    );
    assert_eq!(
        result.get("stdout_offset").and_then(Value::as_usize),
        Some(0)
    );
}

#[test]
fn plain_terminal_requests_stop_without_touching_history_or_poll_cursor() {
    let directory = Directory::new();
    let launch = HttpFixture::new(vec![
        (
            200,
            completion(
                "",
                vec![tool_call(
                    "bash-1",
                    "bash",
                    Value::object([
                        (
                            "command",
                            Value::string(
                                "printf 'ready\\n'; until [ -e go ]; do sleep 0.02; done",
                            ),
                        ),
                        ("wait_ms", Value::number(0)),
                    ]),
                )],
            ),
        ),
        (200, completion("Started.", vec![])),
    ]);
    let mut agent = Agent::new(
        OpenRouter::fixture(launch.endpoint.clone()),
        Tools::new(directory.path()).unwrap(),
    );
    turn(&mut agent, &launch, "Start");
    let first = agent
        .archive()
        .messages
        .lock()
        .unwrap()
        .iter()
        .find(|message| message.get("tool_call_id").and_then(Value::as_str) == Some("bash-1"))
        .and_then(|message| message.get("content").and_then(Value::as_str))
        .map(|content| crate::json::parse(content).unwrap())
        .unwrap();
    let observer = agent.processes();
    let id = observer.processes().unwrap()[0].id.clone();
    let started = Instant::now();
    while observer.read(&id, 0, Window::Tail).unwrap().text != "ready\n" {
        assert!(started.elapsed() < Duration::from_secs(10));
        std::thread::sleep(Duration::from_millis(20));
    }
    let messages = agent.archive().messages.lock().unwrap().len();
    let events = agent.archive().events.lock().unwrap().len();
    let current = printed(&agent, &format!("STOP {id}"));
    assert!(
        current == format!("Stop requested for {id}; awaiting final status.\n")
            || current == format!("{id} · stopped\n"),
        "{current}"
    );
    let list = printed(&agent, "");
    assert!(
        list.contains(&format!("{id}  running"))
            || list == "No active background processes in this conversation.\n",
        "{list}"
    );
    assert!(!list.contains("stopping"), "{list}");
    assert!(!agent.cancellation().requested());
    assert_eq!(agent.archive().messages.lock().unwrap().len(), messages);
    assert_eq!(agent.archive().events.lock().unwrap().len(), events);
    assert_eq!(
        print(&agent, "stop", &mut Vec::new()).unwrap_err(),
        "Usage: /terminal stop PROCESS_ID"
    );
    while observer.processes().unwrap()[0].active() {
        assert!(started.elapsed() < Duration::from_secs(10));
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(
        printed(&agent, ""),
        "No active background processes in this conversation.\n"
    );
    assert!(printed(&agent, &id).contains("Output:\nready\n"));
    assert_eq!(launch.finish().len(), 2);
    let poll = HttpFixture::new(vec![
        (
            200,
            completion(
                "",
                vec![tool_call(
                    "poll-1",
                    "process",
                    Value::object([
                        ("action", Value::string("poll")),
                        ("process_id", Value::string(&id)),
                    ]),
                )],
            ),
        ),
        (200, completion("Polled.", vec![])),
    ]);
    turn(&mut agent, &poll, "Poll");
    let result = agent
        .archive()
        .messages
        .lock()
        .unwrap()
        .iter()
        .find(|message| message.get("tool_call_id").and_then(Value::as_str) == Some("poll-1"))
        .and_then(|message| message.get("content").and_then(Value::as_str))
        .map(|content| crate::json::parse(content).unwrap())
        .unwrap();
    let collected = first
        .get("stdout")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned()
        + result.get("stdout").and_then(Value::as_str).unwrap_or("");
    assert_eq!(collected, "ready\n");
    assert_eq!(
        result.get("stdout_offset").and_then(Value::as_usize),
        first.get("stdout_next_offset").and_then(Value::as_usize)
    );
}
