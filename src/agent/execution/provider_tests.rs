use super::*;
use crate::{
    auth::openai::Session as AuthSession,
    backend::{
        openai::{Api, OpenAi},
        openrouter::OpenRouter,
    },
    checkpoints::session::Session as CheckpointSession,
    test_support::{Directory, HttpFixture, Response, completion, tool_call},
    tools::Tools,
};
use std::{fs, time::Duration};

fn operations() -> Vec<(&'static str, &'static str, Value)> {
    vec![
        (
            "create",
            "write",
            Value::object([
                ("path", Value::string("created.txt")),
                ("content", Value::string("created content\n")),
                ("parallel", Value::Bool(true)),
            ]),
        ),
        (
            "update",
            "edit",
            Value::object([
                ("path", Value::string("existing.txt")),
                (
                    "edits",
                    Value::Array(vec![Value::object([
                        ("old_text", Value::string("before")),
                        ("new_text", Value::string("after")),
                    ])]),
                ),
                ("parallel", Value::Bool(true)),
            ]),
        ),
        (
            "read_created",
            "read",
            Value::object([
                ("path", Value::string("created.txt")),
                ("parallel", Value::Bool(false)),
            ]),
        ),
        (
            "read_updated",
            "read",
            Value::object([("path", Value::string("existing.txt"))]),
        ),
        (
            "invalid_parallel",
            "read",
            Value::object([
                ("path", Value::string("existing.txt")),
                ("parallel", Value::string("true")),
            ]),
        ),
    ]
}

fn openai_completed(output: Vec<Value>) -> Response {
    let event = Value::object([
        ("type", Value::string("response.completed")),
        (
            "response",
            Value::object([
                ("status", Value::string("completed")),
                ("output", Value::Array(output)),
            ]),
        ),
    ]);
    Response::Stream(vec![(
        Duration::ZERO,
        format!("data: {}\n\n", event.encode()),
    )])
}

fn openai_call(id: &str, name: &str, arguments: Value) -> Value {
    Value::object([
        ("type", Value::string("function_call")),
        ("namespace", Value::string("jecode")),
        ("call_id", Value::string(id)),
        ("name", Value::string(name)),
        ("arguments", Value::string(arguments.encode())),
    ])
}

fn openai_answer(text: &str) -> Value {
    Value::object([
        ("type", Value::string("message")),
        ("role", Value::string("assistant")),
        ("phase", Value::string("final_answer")),
        (
            "content",
            Value::Array(vec![Value::object([
                ("type", Value::string("output_text")),
                ("text", Value::string(text)),
            ])]),
        ),
    ])
}

fn openai_agent(project: &Directory, endpoint: &str) -> Agent {
    let api = Api::fixture(
        AuthSession::fixture("isolated-access-token"),
        endpoint.trim_end_matches("/chat/completions"),
    );
    let backend = OpenAi::with_api(api, "fixture-model".into()).unwrap();
    Agent::with_backend(Box::new(backend), Tools::new(project.path()).unwrap())
}

fn prepare(agent: &mut Agent, home: &Directory, project: &Directory) {
    agent.enable_sessions(home.path()).unwrap();
    agent.use_checkpoints(CheckpointSession::offline(home.path(), project.path()));
}

fn tool_results(messages: &[Value]) -> Vec<(&str, Value)> {
    messages
        .iter()
        .filter(|message| message.get("role").and_then(Value::as_str) == Some("tool"))
        .map(|message| {
            (
                message.get("tool_call_id").and_then(Value::as_str).unwrap(),
                json::parse(message.get("content").and_then(Value::as_str).unwrap()).unwrap(),
            )
        })
        .collect()
}

fn assert_completed(agent: &Agent, project: &Directory, events: &[String]) -> String {
    assert_eq!(
        fs::read_to_string(project.path().join("created.txt")).unwrap(),
        "created content\n"
    );
    assert_eq!(
        fs::read_to_string(project.path().join("existing.txt")).unwrap(),
        "after\n"
    );
    assert!(!agent.sessions().unwrap().snapshot().pending.active);
    let events = events.iter().map(String::as_str).collect::<Vec<_>>();
    assert_eq!(&events[..2], &["start:create", "start:update"]);
    let mut finishes = events[2..4].to_vec();
    finishes.sort_unstable();
    assert_eq!(finishes, ["finish:create", "finish:update"]);
    assert_eq!(
        &events[4..],
        &[
            "start:read_created",
            "finish:read_created",
            "start:read_updated",
            "finish:read_updated",
            "start:invalid_parallel",
            "finish:invalid_parallel",
        ]
    );
    let saved = agent.sessions().unwrap().snapshot();
    let results = tool_results(&saved.messages);
    assert_eq!(
        results.iter().map(|(id, _)| *id).collect::<Vec<_>>(),
        [
            "create",
            "update",
            "read_created",
            "read_updated",
            "invalid_parallel"
        ]
    );
    for (_, result) in &results[..2] {
        assert!(result.get("bytes_written").is_some(), "{result:?}");
        let checkpoint = result.get("checkpoint").unwrap();
        assert_eq!(
            checkpoint.get("status").and_then(Value::as_str),
            Some("saved")
        );
        assert!(checkpoint.get("ref").and_then(Value::as_str).is_some());
    }
    assert!(results[2].1.encode().contains("created content"));
    assert!(results[3].1.encode().contains("after"));
    assert_eq!(
        results[4].1.get("error").and_then(Value::as_str),
        Some("parallel must be a boolean")
    );
    let changes = saved
        .events
        .iter()
        .filter(|event| event.get("type").and_then(Value::as_str) == Some("tool_change"))
        .collect::<Vec<_>>();
    assert_eq!(changes.len(), 2);
    for (change, id) in changes.iter().zip(["create", "update"]) {
        assert_eq!(change.get("tool_call_id").and_then(Value::as_str), Some(id));
        assert!(
            !change
                .get("change")
                .unwrap()
                .get("hunks")
                .unwrap()
                .as_array()
                .unwrap()
                .is_empty()
        );
    }
    assert_eq!(
        changes[0].get("change").unwrap().get("created"),
        Some(&Value::Bool(true))
    );
    assert_eq!(
        changes[1].get("change").unwrap().get("created"),
        Some(&Value::Bool(false))
    );
    assert!(
        changes[0]
            .get("after_message")
            .and_then(Value::as_usize)
            .unwrap()
            < changes[1]
                .get("after_message")
                .and_then(Value::as_usize)
                .unwrap()
    );
    agent.sessions().unwrap().id()
}

fn event_order(agent: &mut Agent) -> Vec<String> {
    let mut events = Vec::new();
    agent
        .run_turn(
            "Make independent changes, then inspect them.",
            &mut |event| {
                match event {
                    Event::ToolStarted { id, .. } => events.push(format!("start:{id}")),
                    Event::ToolFinished { id, .. } => events.push(format!("finish:{id}")),
                    _ => {}
                }
                Ok(())
            },
        )
        .unwrap();
    events
}

#[test]
fn openrouter_parallel_arguments_keep_results_changes_and_resume_order() {
    let project = Directory::new();
    let home = Directory::new();
    fs::write(project.path().join("existing.txt"), "before\n").unwrap();
    let calls = operations()
        .into_iter()
        .map(|(id, name, arguments)| tool_call(id, name, arguments))
        .collect();
    let fixture = HttpFixture::new(vec![
        (200, completion("", calls)),
        (200, completion("Changes complete.", vec![])),
        (200, completion("Continued.", vec![])),
    ]);
    let mut first = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(project.path()).unwrap(),
    );
    prepare(&mut first, &home, &project);
    let events = event_order(&mut first);
    let id = assert_completed(&first, &project, &events);
    first.sessions().unwrap().flush().unwrap();
    drop(first);

    let mut resumed = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(project.path()).unwrap(),
    );
    resumed.enable_sessions(home.path()).unwrap();
    resumed.resume(&id).unwrap();
    assert_eq!(
        tool_results(&resumed.sessions().unwrap().snapshot().messages).len(),
        5
    );
    resumed.run_turn("Continue.", &mut |_| Ok(())).unwrap();
    let requests = fixture.finish();
    assert_eq!(requests.len(), 3);
    let next = requests[1]
        .body
        .get("messages")
        .and_then(Value::as_array)
        .unwrap();
    assert_eq!(
        tool_results(next)
            .iter()
            .map(|(id, _)| *id)
            .collect::<Vec<_>>(),
        [
            "create",
            "update",
            "read_created",
            "read_updated",
            "invalid_parallel"
        ]
    );
    assert!(requests[2].body.encode().contains("Changes complete."));
    assert_eq!(
        fs::read_to_string(project.path().join("existing.txt")).unwrap(),
        "after\n"
    );
}

#[test]
fn openai_parallel_arguments_keep_results_changes_and_resume_order() {
    let project = Directory::new();
    let home = Directory::new();
    fs::write(project.path().join("existing.txt"), "before\n").unwrap();
    let calls = operations()
        .into_iter()
        .map(|(id, name, arguments)| openai_call(id, name, arguments))
        .collect();
    let fixture = HttpFixture::streaming(vec![
        openai_completed(calls),
        openai_completed(vec![openai_answer("Changes complete.")]),
        openai_completed(vec![openai_answer("Continued.")]),
    ]);
    let mut first = openai_agent(&project, &fixture.endpoint);
    prepare(&mut first, &home, &project);
    let events = event_order(&mut first);
    let id = assert_completed(&first, &project, &events);
    first.sessions().unwrap().flush().unwrap();
    drop(first);

    let mut resumed = openai_agent(&project, &fixture.endpoint);
    resumed.enable_sessions(home.path()).unwrap();
    resumed.resume(&id).unwrap();
    assert_eq!(
        tool_results(&resumed.sessions().unwrap().snapshot().messages).len(),
        5
    );
    resumed.run_turn("Continue.", &mut |_| Ok(())).unwrap();
    let requests = fixture.finish();
    assert_eq!(requests.len(), 3);
    let next = requests[1]
        .body
        .get("input")
        .and_then(Value::as_array)
        .unwrap();
    let outputs = next
        .iter()
        .filter(|item| item.get("type").and_then(Value::as_str) == Some("function_call_output"))
        .collect::<Vec<_>>();
    assert_eq!(
        outputs
            .iter()
            .map(|item| item.get("call_id").and_then(Value::as_str).unwrap())
            .collect::<Vec<_>>(),
        [
            "create",
            "update",
            "read_created",
            "read_updated",
            "invalid_parallel"
        ]
    );
    assert!(requests[2].body.encode().contains("Changes complete."));
    assert_eq!(
        fs::read_to_string(project.path().join("existing.txt")).unwrap(),
        "after\n"
    );
}
