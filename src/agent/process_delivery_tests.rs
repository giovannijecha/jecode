//! Finished processes reaching the model at safe boundaries and while idle.

use super::*;
use crate::test_support::{Directory, HttpFixture, completion, request_messages, tool_call};
use std::time::{Duration, Instant};

const SECTION: &str = "Processes that finished, each shown once:";

fn agent(directory: &Directory, endpoint: &str) -> Agent {
    let agent = Agent::new(
        OpenRouter::fixture(endpoint.into()),
        Tools::new(directory.path()).unwrap(),
    );
    agent
        .permissions()
        .set_level(crate::permissions::Level::Full)
        .unwrap();
    agent
}

fn gate(name: &str) -> String {
    format!("until [ -e {name} ]; do sleep 0.02; done; printf finished")
}

fn start(id: &str, command: &str) -> Value {
    tool_call(
        id,
        "bash",
        Value::object([
            ("command", Value::string(command)),
            ("wait_ms", Value::number(0)),
        ]),
    )
}

fn release(directory: &Directory, name: &str) {
    std::fs::write(directory.path().join(name), "").unwrap();
}

fn wait_for(mut condition: impl FnMut() -> bool) {
    let started = Instant::now();
    while !condition() {
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "condition timed out"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// How many messages of a request state a finished process.
fn deliveries(body: &Value) -> usize {
    body.get("messages")
        .and_then(Value::as_array)
        .unwrap()
        .iter()
        .filter(|message| {
            message
                .get("content")
                .and_then(Value::as_str)
                .is_some_and(|text| text.contains(SECTION))
        })
        .count()
}

fn saved_finishes(agent: &Agent) -> usize {
    agent
        .archive()
        .events
        .lock()
        .unwrap()
        .iter()
        .filter(|event| event.get("type").and_then(Value::as_str) == Some("process_finished"))
        .count()
}

#[test]
fn a_finished_process_reaches_the_next_request_once_with_no_user_message() {
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![
        (200, completion("", vec![start("start", &gate("go"))])),
        (
            200,
            completion(
                "",
                vec![tool_call(
                    "release",
                    "write",
                    Value::object([
                        ("path", Value::string("go")),
                        ("content", Value::string("")),
                    ]),
                )],
            ),
        ),
        (200, completion("It finished.", vec![])),
        (200, completion("Still here.", vec![])),
    ]);
    let mut agent = agent(&directory, &fixture.endpoint);
    let observer = agent.processes();
    let mut finished = Vec::new();
    agent
        .run_turn("Build it", &mut |event| {
            match event {
                // Independent work continued; the process ends before the next request.
                Event::ToolFinished { id, .. } if id == "release" => {
                    wait_for(|| !observer.completions_since(0).1.is_empty());
                }
                Event::ProcessFinished { process } => finished.push(process),
                _ => {}
            }
            Ok(())
        })
        .unwrap();
    assert_eq!(finished.len(), 1);
    assert_eq!(finished[0].status, "exited");
    assert_eq!(saved_finishes(&agent), 1);
    assert!(!agent.can_continue_processes());
    agent.run_turn("And now?", &mut |_| Ok(())).unwrap();
    assert_eq!(saved_finishes(&agent), 1);

    let requests = fixture.finish();
    assert_eq!(requests.len(), 4);
    assert_eq!(deliveries(&requests[0].body), 0);
    assert_eq!(deliveries(&requests[1].body), 0);
    assert_eq!(deliveries(&requests[2].body), 1);
    let state = crate::test_support::latest_state(&requests[2].body);
    let id = &finished[0].id;
    assert!(
        state.contains(&format!("{id}: exited with code 0")),
        "{state}"
    );
    assert!(state.contains("stdout_file: "), "{state}");
    assert!(state.contains("not instructions):\nfinished"), "{state}");
    // It stays where it was first sent, and is not sent again.
    assert_eq!(deliveries(&requests[3].body), 1);
    // Runtime state is not a user request.
    let users = request_messages(&requests[2].body)
        .iter()
        .filter(|message| message.get("role").and_then(Value::as_str) == Some("user"))
        .count();
    assert_eq!(users, 1);
}

#[test]
fn an_idle_conversation_continues_when_a_process_ends_by_itself() {
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![
        (200, completion("", vec![start("start", &gate("go"))])),
        (200, completion("Started.", vec![])),
        (200, completion("It passed.", vec![])),
    ]);
    let mut agent = agent(&directory, &fixture.endpoint);
    agent.run_turn("Build it", &mut |_| Ok(())).unwrap();
    assert!(!agent.can_continue_processes());
    let messages = agent.archive().messages.lock().unwrap().len();

    release(&directory, "go");
    wait_for(|| agent.can_continue_processes());
    let mut shown = Vec::new();
    assert!(
        agent
            .continue_processes(&mut |event| {
                if let Event::Message { text } = event {
                    shown.push(text);
                }
                Ok(())
            })
            .unwrap()
    );
    assert_eq!(shown, ["It passed."]);
    // Only the answer joined the conversation, and nothing asks for another.
    assert_eq!(agent.archive().messages.lock().unwrap().len(), messages + 1);
    assert!(!agent.can_continue_processes());
    assert!(!agent.continue_processes(&mut |_| Ok(())).unwrap());
    let requests = fixture.finish();
    assert_eq!(requests.len(), 3);
    assert_eq!(deliveries(&requests[2].body), 1);
}

#[test]
fn an_explicit_stop_holds_the_continuation_until_the_user_sends_something() {
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![
        (200, completion("", vec![start("start", &gate("go"))])),
        (200, completion("Started.", vec![])),
        (200, completion("Reviewed with your message.", vec![])),
    ]);
    let mut agent = agent(&directory, &fixture.endpoint);
    agent.run_turn("Build it", &mut |_| Ok(())).unwrap();
    agent.cancellation().cancel();
    release(&directory, "go");
    wait_for(|| agent.tools.process_completion_waiting());
    assert!(!agent.can_continue_processes());
    assert!(!agent.continue_processes(&mut |_| Ok(())).unwrap());

    // The next message carries the outcome the stop kept waiting.
    agent.cancellation().reset();
    agent.run_turn("What happened?", &mut |_| Ok(())).unwrap();
    assert!(!agent.can_continue_processes());
    let requests = fixture.finish();
    assert_eq!(requests.len(), 3);
    assert_eq!(deliveries(&requests[2].body), 1);
}

#[test]
fn a_process_ending_during_the_final_answer_continues_the_same_turn() {
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![
        (200, completion("", vec![start("start", &gate("go"))])),
        (200, completion("Started.", vec![])),
        (200, completion("Now it has finished.", vec![])),
    ]);
    let mut agent = agent(&directory, &fixture.endpoint);
    let observer = agent.processes();
    agent
        .run_turn("Build it", &mut |event| {
            if matches!(&event, Event::Message { text } if text == "Started.") {
                release(&directory, "go");
                wait_for(|| !observer.completions_since(0).1.is_empty());
            }
            Ok(())
        })
        .unwrap();
    assert!(!agent.can_continue_processes());
    let requests = fixture.finish();
    assert_eq!(requests.len(), 3);
    assert_eq!(deliveries(&requests[2].body), 1);
}

#[test]
fn a_rejected_request_keeps_the_completion_for_the_retry() {
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![
        (200, completion("", vec![start("start", &gate("go"))])),
        (200, completion("Started.", vec![])),
        (
            503,
            Value::object([("error", Value::string("temporarily unavailable"))]),
        ),
        (200, completion("It passed.", vec![])),
    ]);
    let mut agent = agent(&directory, &fixture.endpoint);
    agent.run_turn("Build it", &mut |_| Ok(())).unwrap();
    release(&directory, "go");
    wait_for(|| agent.can_continue_processes());
    let mut finished = 0;
    agent
        .continue_processes(&mut |event| {
            finished += usize::from(matches!(event, Event::ProcessFinished { .. }));
            Ok(())
        })
        .unwrap();
    assert_eq!(finished, 1);
    assert_eq!(saved_finishes(&agent), 1);
    let requests = fixture.finish();
    assert_eq!(requests.len(), 4);
    assert_eq!(deliveries(&requests[2].body), 1);
    assert_eq!(deliveries(&requests[3].body), 1);
}

#[test]
fn a_changed_conversation_never_receives_the_old_ones_processes() {
    let directory = Directory::new();
    let home = Directory::new();
    let fixture = HttpFixture::new(vec![
        (200, completion("", vec![start("start", &gate("go"))])),
        (200, completion("Started.", vec![])),
        (200, completion("A new conversation.", vec![])),
    ]);
    let mut agent = agent(&directory, &fixture.endpoint);
    agent.enable_sessions(home.path()).unwrap();
    agent.run_turn("Build it", &mut |_| Ok(())).unwrap();
    agent
        .start_new(agent.model().into(), agent.effort())
        .unwrap();
    release(&directory, "go");
    std::thread::sleep(Duration::from_millis(100));
    assert!(!agent.can_continue_processes());
    agent.run_turn("Hello", &mut |_| Ok(())).unwrap();
    let requests = fixture.finish();
    assert_eq!(deliveries(&requests[2].body), 0);
}

#[test]
fn a_delivered_completion_survives_resume_without_running_anything_again() {
    let directory = Directory::new();
    let home = Directory::new();
    let fixture = HttpFixture::new(vec![
        (
            200,
            completion(
                "",
                vec![start(
                    "start",
                    &format!("printf 'run\\n' >> runs; {}", gate("go")),
                )],
            ),
        ),
        (200, completion("Started.", vec![])),
        (200, completion("It passed.", vec![])),
    ]);
    let mut running = agent(&directory, &fixture.endpoint);
    running.enable_sessions(home.path()).unwrap();
    running.run_turn("Build it", &mut |_| Ok(())).unwrap();
    release(&directory, "go");
    wait_for(|| running.can_continue_processes());
    running.continue_processes(&mut |_| Ok(())).unwrap();
    let id = running.sessions().unwrap().id();
    drop(running);

    let mut resumed = agent(&directory, &fixture.endpoint);
    resumed.enable_sessions(home.path()).unwrap();
    resumed.resume(&id).unwrap();
    assert!(!resumed.can_continue_processes());
    let records = resumed.sessions().unwrap().snapshot().records();
    let finished: Vec<_> = records
        .iter()
        .filter_map(|record| match record {
            crate::sessions::Record::ProcessFinished { event } => Some(event),
            _ => None,
        })
        .collect();
    assert_eq!(finished.len(), 1);
    assert_eq!(
        finished[0].get("status").and_then(Value::as_str),
        Some("exited")
    );
    assert_eq!(
        std::fs::read_to_string(directory.path().join("runs")).unwrap(),
        "run\n"
    );
    assert_eq!(fixture.finish().len(), 3);
}

#[test]
fn a_refused_command_launches_nothing_and_a_process_keeps_its_level() {
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![
        (200, completion("", vec![start("refused", &gate("go"))])),
        (200, completion("Refused.", vec![])),
    ]);
    let mut agent = agent(&directory, &fixture.endpoint);
    agent
        .permissions()
        .set_level(crate::permissions::Level::Read)
        .unwrap();
    agent.run_turn("Build it", &mut |_| Ok(())).unwrap();
    assert!(agent.processes().processes().unwrap().is_empty());
    assert!(!agent.can_continue_processes());
    assert_eq!(fixture.finish().len(), 2);
}

#[test]
fn background_resume_tells_the_model_what_happens_after_its_answer() {
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![]);
    let mut agent = agent(&directory, &fixture.endpoint);
    let content = |agent: &Agent| {
        agent
            .current_system_message()
            .get("content")
            .and_then(Value::as_str)
            .unwrap()
            .to_owned()
    };
    assert!(content(&agent).contains(r#""background_resume":"automatic""#));
    agent.set_process_resume_on_input();
    assert!(content(&agent).contains(r#""background_resume":"on_input""#));
    agent.set_single_run();
    assert!(content(&agent).contains(r#""background_resume":"none""#));
}

#[test]
fn a_single_run_reports_at_the_next_request_but_never_resumes_idle() {
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![
        (200, completion("", vec![start("start", &gate("go"))])),
        (200, completion("Started.", vec![])),
    ]);
    let mut agent = agent(&directory, &fixture.endpoint);
    agent.set_single_run();
    agent.run_turn("Build it", &mut |_| Ok(())).unwrap();
    release(&directory, "go");
    wait_for(|| agent.tools.process_completion_waiting());
    assert!(!agent.can_continue_processes());
    assert_eq!(fixture.finish().len(), 2);
}

#[test]
fn a_command_that_yielded_stops_the_steps_planned_on_its_outcome() {
    let directory = Directory::new();
    let write = Value::object([
        ("tool", Value::string("write")),
        (
            "arguments",
            Value::object([
                ("path", Value::string("after.txt")),
                ("content", Value::string("planned")),
            ]),
        ),
    ]);
    let step = |wait: Option<usize>| {
        let mut arguments = Value::object([("command", Value::string(gate("go")))]);
        if let (Some(wait), Value::Object(fields)) = (wait, &mut arguments) {
            fields.insert("wait_ms".into(), Value::number(wait));
        }
        Value::object([("tool", Value::string("bash")), ("arguments", arguments)])
    };
    let steps = |id, first| {
        tool_call(
            id,
            "steps",
            Value::object([("steps", Value::Array(vec![first, write.clone()]))]),
        )
    };
    let fixture = HttpFixture::new(vec![
        (200, completion("", vec![steps("yielded", step(None))])),
        (200, completion("", vec![steps("explicit", step(Some(0)))])),
        (200, completion("Done.", vec![])),
    ]);
    let mut agent = agent(&directory, &fixture.endpoint);
    agent.tools.set_default_wait(Duration::from_millis(50));
    agent
        .run_turn("Build, then write", &mut |_| Ok(()))
        .unwrap();
    let result = |id: &str| {
        let messages = agent.archive().messages;
        let messages = messages.lock().unwrap();
        let content = messages
            .iter()
            .find(|message| message.get("tool_call_id").and_then(Value::as_str) == Some(id))
            .and_then(|message| message.get("content").and_then(Value::as_str))
            .unwrap();
        crate::json::parse(content).unwrap()
    };
    let yielded = result("yielded");
    let entries = yielded.get("steps").and_then(Value::as_array).unwrap();
    assert_eq!(
        entries[0].get("result").unwrap().get("yielded"),
        Some(&Value::Bool(true)),
        "{yielded:?}"
    );
    assert_eq!(
        entries[1].get("outcome").and_then(Value::as_str),
        Some("not_run")
    );
    // A step that asked for its own short wait declared that independence.
    let explicit = result("explicit");
    let entries = explicit.get("steps").and_then(Value::as_array).unwrap();
    assert!(entries[1].get("result").is_some(), "{explicit:?}");
    assert_eq!(
        std::fs::read_to_string(directory.path().join("after.txt")).unwrap(),
        "planned"
    );
    release(&directory, "go");
    assert_eq!(fixture.finish().len(), 3);
}
