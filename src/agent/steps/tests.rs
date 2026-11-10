use super::*;
use crate::{
    backend::openrouter::OpenRouter,
    test_support::{Directory, HttpFixture, completion, tool_call},
    tools::Tools,
};
use std::fs;

mod integrations;
mod interruption;

#[test]
fn a_stopped_command_fails_its_step_and_a_requested_stop_does_not() {
    let stopped = Value::object([
        ("process_id", Value::string("process-1")),
        ("status", Value::string("stopped")),
    ]);
    assert!(failed("bash", &stopped));
    assert!(failed("exec", &stopped));
    assert!(!failed("process", &stopped));
}

fn step(tool: &str, arguments: Value) -> Value {
    Value::object([("tool", Value::string(tool)), ("arguments", arguments)])
}

fn write(path: &str, content: &str) -> Value {
    step(
        "write",
        Value::object([
            ("path", Value::string(path)),
            ("content", Value::string(content)),
        ]),
    )
}

fn read(path: &str) -> Value {
    step("read", Value::object([("path", Value::string(path))]))
}

fn agent(project: &Directory, fixture: &HttpFixture) -> Agent {
    Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(project.path()).unwrap(),
    )
}

fn fixture(steps: Vec<Value>) -> HttpFixture {
    HttpFixture::new(vec![
        (
            200,
            completion(
                "",
                vec![tool_call(
                    "chain",
                    "steps",
                    Value::object([("steps", Value::Array(steps))]),
                )],
            ),
        ),
        (200, completion("Finished", vec![])),
    ])
}

/// The tool messages the turn recorded, with their parsed results.
fn results(agent: &Agent) -> Vec<(String, Value)> {
    agent
        .messages
        .lock()
        .unwrap()
        .iter()
        .filter(|message| message.get("role").and_then(Value::as_str) == Some("tool"))
        .map(|message| {
            (
                message
                    .get("tool_call_id")
                    .and_then(Value::as_str)
                    .unwrap()
                    .to_string(),
                json::parse(message.get("content").and_then(Value::as_str).unwrap()).unwrap(),
            )
        })
        .collect()
}

fn entries(result: &Value) -> &[Value] {
    result.get("steps").and_then(Value::as_array).unwrap()
}

#[test]
fn steps_run_in_order_and_answer_the_one_call() {
    let project = Directory::new();
    let fixture = fixture(vec![write("a.txt", "first"), read("a.txt")]);
    let mut agent = agent(&project, &fixture);
    agent.run_turn("Do it", &mut |_| Ok(())).unwrap();

    let results = results(&agent);
    assert_eq!(results.len(), 1, "one call has one result: {results:?}");
    assert_eq!(results[0].0, "chain");
    let steps = entries(&results[0].1);
    assert_eq!(steps.len(), 2);
    assert_eq!(steps[0].get("tool").and_then(Value::as_str), Some("write"));
    assert!(
        steps[1].get("result").unwrap().encode().contains("first"),
        "the second step saw the first step's file: {steps:?}"
    );
}

#[test]
fn a_long_prepared_sequence_keeps_every_write_and_saved_change() {
    let project = Directory::new();
    let home = Directory::new();
    let fixture = fixture(
        (0..17)
            .map(|index| write(&format!("file-{index}.txt"), "prepared\n"))
            .collect(),
    );
    let mut agent = agent(&project, &fixture);
    agent.enable_sessions(home.path()).unwrap();
    agent
        .run_turn("Create the prepared files", &mut |_| Ok(()))
        .unwrap();
    let results = results(&agent);
    assert!(results[0].1.get("error").is_none(), "{:?}", results[0]);
    assert_eq!(entries(&results[0].1).len(), 17);
    for index in 0..17 {
        assert_eq!(
            fs::read_to_string(project.path().join(format!("file-{index}.txt"))).unwrap(),
            "prepared\n"
        );
    }
    let saved = agent.sessions().unwrap().snapshot().records();
    let changes = saved
        .iter()
        .filter(|record| {
            matches!(
                record,
                crate::sessions::Record::Tool {
                    change: Some(_),
                    ..
                }
            )
        })
        .count();
    assert_eq!(changes, 17, "early changes must survive a long sequence");
}

#[test]
fn a_media_read_in_steps_asks_to_be_read_alone() {
    let call = ToolCall {
        id: "chain_s1".into(),
        name: "read".into(),
        arguments: "{}".into(),
    };
    let media = Value::object([("view", Value::string("image"))]);
    let answer = combined(&[call], &[("chain_s1".into(), media)]);
    let result = entries(&answer)[0].get("result").unwrap();
    assert!(result.get("view").is_none(), "no view would ever follow");
    assert!(result.encode().contains("read it alone"), "{result:?}");
}

#[test]
fn a_saved_session_shows_a_card_per_step_that_ran_with_its_change() {
    let project = Directory::new();
    let home = Directory::new();
    let fixture = fixture(vec![
        write("a.txt", "first"),
        read("missing.txt"),
        write("b.txt", "never"),
    ]);
    let mut agent = agent(&project, &fixture);
    agent.enable_sessions(home.path()).unwrap();
    agent.run_turn("Do it", &mut |_| Ok(())).unwrap();
    let cards = agent
        .sessions()
        .unwrap()
        .snapshot()
        .records()
        .into_iter()
        .filter_map(|record| match record {
            crate::sessions::Record::Tool {
                id, name, change, ..
            } => Some((id, name, change.is_some())),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        cards,
        [
            ("chain_s1".to_string(), "write".to_string(), true),
            ("chain_s2".to_string(), "read".to_string(), false),
        ]
    );
}

fn plan(status: &str) -> Value {
    step(
        "todo_list",
        Value::object([
            ("action", Value::string("set")),
            ("title", Value::string("Ship")),
            (
                "tasks",
                Value::Array(vec![Value::object([
                    ("id", Value::string("t1")),
                    ("title", Value::string("Build")),
                    ("status", Value::string(status)),
                ])]),
            ),
        ]),
    )
}

#[test]
fn a_plan_set_in_steps_is_current_also_after_resume() {
    let project = Directory::new();
    let home = Directory::new();
    let fixture = fixture(vec![plan("pending"), read("missing.txt"), plan("never")]);
    let mut first = agent(&project, &fixture);
    first.enable_sessions(home.path()).unwrap();
    first.run_turn("Do it", &mut |_| Ok(())).unwrap();
    let plan = first.todo_plan().expect("the plan is known");
    assert_eq!(plan.revision, 1);
    assert_eq!(plan.tasks[0].status, "pending");
    assert!(first.request_environment().contains("revision 1"));
    let id = first.sessions().unwrap().id();
    drop(first);

    let mut resumed = agent(&project, &fixture);
    resumed.enable_sessions(home.path()).unwrap();
    resumed.resume(&id).unwrap();
    assert_eq!(resumed.todo_plan().unwrap(), plan);
}

#[test]
fn a_second_plan_in_one_call_builds_on_the_first() {
    let project = Directory::new();
    let fixture = fixture(vec![plan("in_progress"), plan("done")]);
    let mut agent = agent(&project, &fixture);
    agent.run_turn("Do it", &mut |_| Ok(())).unwrap();
    let plan = agent.todo_plan().unwrap();
    assert_eq!(plan.revision, 2);
    assert_eq!(plan.tasks[0].status, "done");
}

#[test]
fn a_skill_read_in_steps_counts_as_delivered() {
    let result = Value::object([(
        "steps",
        Value::Array(vec![Value::object([
            ("tool", Value::string("read")),
            (
                "result",
                Value::object([("skill_reference", Value::string("skill:review"))]),
            ),
        ])]),
    )]);
    assert_eq!(
        super::super::skills::delivered(&result).collect::<Vec<_>>(),
        ["review"]
    );
}

#[test]
fn a_step_the_level_refuses_stops_the_rest_without_running_them() {
    let project = Directory::new();
    let fixture = fixture(vec![write("a.txt", "no"), write("b.txt", "no")]);
    let mut agent = agent(&project, &fixture);
    agent
        .permissions()
        .set_level(crate::permissions::Level::Read)
        .unwrap();
    agent.run_turn("Do it", &mut |_| Ok(())).unwrap();
    let results = results(&agent);
    let steps = entries(&results[0].1);
    assert_eq!(
        steps[0]
            .get("result")
            .unwrap()
            .get("outcome")
            .and_then(Value::as_str),
        Some("unavailable")
    );
    assert_eq!(
        steps[1].get("outcome").and_then(Value::as_str),
        Some("not_run")
    );
    assert!(!project.path().join("a.txt").exists());
    assert!(!project.path().join("b.txt").exists());
}

#[test]
fn an_interface_failure_mid_chain_still_answers_the_call() {
    let project = Directory::new();
    let home = Directory::new();
    let fixture = HttpFixture::new(vec![(
        200,
        completion(
            "",
            vec![tool_call(
                "chain",
                "steps",
                Value::object([(
                    "steps",
                    Value::Array(vec![write("a.txt", "first"), write("b.txt", "second")]),
                )]),
            )],
        ),
    )]);
    let mut agent = agent(&project, &fixture);
    agent.enable_sessions(home.path()).unwrap();
    let mut started = 0;
    let failure = agent
        .run_turn("Do it", &mut |event| {
            if matches!(event, crate::events::Event::ToolStarted { .. }) {
                started += 1;
                if started == 2 {
                    return Err("Interface closed".into());
                }
            }
            Ok(())
        })
        .unwrap_err();
    assert_eq!(failure, "Interface closed");
    let results = results(&agent);
    assert_eq!(results.len(), 1, "the call has its one result: {results:?}");
    let steps = entries(&results[0].1);
    assert!(steps[0].get("result").unwrap().get("error").is_none());
    assert_eq!(
        steps[1]
            .get("result")
            .unwrap()
            .get("outcome")
            .and_then(Value::as_str),
        Some("not_executed")
    );
    assert!(!project.path().join("b.txt").exists());
}

#[test]
fn a_namespaced_tool_name_runs_the_tool() {
    let project = Directory::new();
    let fixture = fixture(vec![step(
        "jecode.write",
        Value::object([
            ("path", Value::string("a.txt")),
            ("content", Value::string("first")),
        ]),
    )]);
    let mut agent = agent(&project, &fixture);
    agent.run_turn("Do it", &mut |_| Ok(())).unwrap();
    assert_eq!(
        fs::read_to_string(project.path().join("a.txt")).unwrap(),
        "first"
    );
}

#[test]
fn a_partly_failed_edit_stops_the_rest_and_a_partial_read_does_not() {
    let project = Directory::new();
    fs::write(project.path().join("a.txt"), "old").unwrap();
    let edits = |path: &str| {
        Value::object([
            ("path", Value::string(path)),
            (
                "edits",
                Value::Array(vec![Value::object([
                    ("old_text", Value::string("old")),
                    ("new_text", Value::string("new")),
                ])]),
            ),
        ])
    };
    let fixture = fixture(vec![
        step(
            "read",
            Value::object([(
                "paths",
                Value::Array(vec![Value::string("missing.txt"), Value::string("a.txt")]),
            )]),
        ),
        step(
            "edit",
            Value::object([(
                "files",
                Value::Array(vec![edits("a.txt"), edits("missing.txt")]),
            )]),
        ),
        write("after.txt", "no"),
    ]);
    let mut agent = agent(&project, &fixture);
    agent.run_turn("Do it", &mut |_| Ok(())).unwrap();
    let results = results(&agent);
    let steps = entries(&results[0].1);
    assert!(steps[0].get("result").is_some());
    assert!(steps[1].get("result").is_some(), "the read went on");
    assert_eq!(
        steps[2].get("outcome").and_then(Value::as_str),
        Some("not_run")
    );
    assert!(!project.path().join("after.txt").exists());
}

#[test]
fn a_failed_step_stops_the_rest() {
    let project = Directory::new();
    let fixture = fixture(vec![read("missing.txt"), write("after.txt", "no")]);
    let mut agent = agent(&project, &fixture);
    agent.run_turn("Do it", &mut |_| Ok(())).unwrap();

    let results = results(&agent);
    let steps = entries(&results[0].1);
    assert!(steps[0].get("result").unwrap().get("error").is_some());
    assert_eq!(
        steps[1].get("outcome").and_then(Value::as_str),
        Some("not_run")
    );
    assert!(!project.path().join("after.txt").exists());
}

#[test]
fn every_step_keeps_its_own_card() {
    let project = Directory::new();
    let fixture = fixture(vec![write("a.txt", "first"), read("a.txt")]);
    let mut agent = agent(&project, &fixture);
    let mut started = Vec::new();
    agent
        .run_turn("Do it", &mut |event| {
            if let crate::events::Event::ToolStarted { id, name, .. } = event {
                started.push((id, name));
            }
            Ok(())
        })
        .unwrap();
    assert_eq!(
        started,
        [
            ("chain_s1".to_string(), "write".to_string()),
            ("chain_s2".to_string(), "read".to_string()),
        ]
    );
}

#[test]
fn steps_that_cannot_run_are_refused_without_running_any() {
    for (steps, expected) in [
        (
            vec![write("a.txt", "no"), step("steps", Value::object([]))],
            "steps cannot run as a step",
        ),
        (
            vec![write("a.txt", "no"), step("ask_user", Value::object([]))],
            "ask_user cannot run as a step",
        ),
        (
            vec![write("a.txt", "no"), step("nonsense", Value::object([]))],
            "Unknown tool: nonsense",
        ),
        (Vec::new(), "steps must contain at least one entry"),
    ] {
        let project = Directory::new();
        let fixture = fixture(steps);
        let mut agent = agent(&project, &fixture);
        agent.run_turn("Do it", &mut |_| Ok(())).unwrap();
        let results = results(&agent);
        assert_eq!(
            results[0].1.get("error").and_then(Value::as_str),
            Some(expected)
        );
        assert!(!project.path().join("a.txt").exists());
    }
}

#[test]
fn a_session_with_steps_and_file_changes_resumes() {
    let project = Directory::new();
    let home = Directory::new();
    let fixture = fixture(vec![write("a.txt", "first"), write("b.txt", "second")]);
    let mut first = agent(&project, &fixture);
    first.enable_sessions(home.path()).unwrap();
    first.run_turn("Do it", &mut |_| Ok(())).unwrap();
    let id = first.sessions().unwrap().id();
    let saved = first.messages.lock().unwrap().clone();
    drop(first);

    let mut resumed = agent(&project, &fixture);
    resumed.enable_sessions(home.path()).unwrap();
    resumed.resume(&id).unwrap();
    assert_eq!(*resumed.messages.lock().unwrap(), saved);
    assert_eq!(
        fs::read_to_string(project.path().join("b.txt")).unwrap(),
        "second"
    );
}
