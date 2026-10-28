use super::*;
use crate::{
    auth::openai::Session,
    backend::openai::{Api, OpenAi},
    json,
    test_support::{Directory, HttpFixture, Response, completion, tool_call},
};
use std::{fs, time::Duration};

fn receipt(messages: &[Value], id: &str) -> Value {
    let message = messages
        .iter()
        .find(|message| message.get("tool_call_id").and_then(Value::as_str) == Some(id))
        .unwrap();
    json::parse(message.get("content").and_then(Value::as_str).unwrap()).unwrap()
}

fn note<'a>(overview: &'a Value, id: &str) -> &'a Value {
    overview
        .get("entries")
        .unwrap()
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry.get("id").and_then(Value::as_str) == Some(id))
        .unwrap()
}

fn source(path: &str, line: usize) -> Value {
    Value::object([("path", Value::string(path)), ("line", Value::number(line))])
}

fn remember() -> Value {
    Value::object([
        ("action", Value::string("remember")),
        ("title", Value::string("Entry point")),
        (
            "summary",
            Value::string("The run function starts the program."),
        ),
        ("sources", Value::Array(vec![source("src/main.rs", 1)])),
    ])
}

#[test]
fn openrouter_map_note_reaches_model_changes_with_source_and_survives_resume() {
    let project = Directory::new();
    let home = Directory::new();
    fs::create_dir(project.path().join("src")).unwrap();
    fs::write(project.path().join("src/main.rs"), "fn run() {}\n").unwrap();
    let fixture = HttpFixture::new(vec![
        (
            200,
            completion(
                "Recording a source-backed note.",
                vec![tool_call("remember-map", "map", remember())],
            ),
        ),
        (200, completion("The note is recorded.", vec![])),
        (
            200,
            completion(
                "Checking the map again.",
                vec![tool_call("changed-map", "map", Value::object([]))],
            ),
        ),
        (200, completion("The source has changed.", vec![])),
    ]);
    let mut agent = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(project.path()).unwrap(),
    );
    agent.enable_sessions(home.path()).unwrap();
    agent
        .run_turn("Remember the entry point", &mut |_| Ok(()))
        .unwrap();
    let id = agent.sessions().unwrap().id();
    fs::write(
        project.path().join("src/main.rs"),
        "fn run() { println!(\"changed\"); }\n",
    )
    .unwrap();
    agent
        .run_turn("Check the project map", &mut |_| Ok(()))
        .unwrap();
    let requests = fixture.finish();
    assert_eq!(requests.len(), 4);
    let catalog = requests[0].body.get("tools").unwrap().as_array().unwrap();
    assert_eq!(
        catalog
            .iter()
            .rev()
            .nth(1)
            .unwrap()
            .get("function")
            .unwrap()
            .get("name")
            .and_then(Value::as_str),
        Some("map")
    );
    let next = requests[1]
        .body
        .get("messages")
        .unwrap()
        .as_array()
        .unwrap();
    let saved = receipt(next, "remember-map");
    let note_id = saved.get("id").and_then(Value::as_str).unwrap().to_owned();
    let system = requests[2]
        .body
        .get("messages")
        .unwrap()
        .as_array()
        .unwrap()[0]
        .get("content")
        .and_then(Value::as_str)
        .unwrap();
    assert!(!system.contains("The run function starts the program."));
    let changed = receipt(
        requests[3]
            .body
            .get("messages")
            .unwrap()
            .as_array()
            .unwrap(),
        "changed-map",
    );
    let entry = note(&changed, &note_id);
    assert_eq!(entry.get("origin").and_then(Value::as_str), Some("model"));
    assert_eq!(
        entry.get("freshness").and_then(Value::as_str),
        Some("changed")
    );
    assert_eq!(
        entry.get("summary").and_then(Value::as_str),
        Some("The run function starts the program.")
    );
    assert_eq!(changed.get("notes_count"), Some(&Value::number(1)));
    assert_eq!(
        receipt(&agent.messages.lock().unwrap(), "changed-map"),
        changed
    );
    drop(agent);

    let resumed_fixture = HttpFixture::new(vec![
        (
            200,
            completion(
                "Looking up the map after resume.",
                vec![tool_call("resumed-map", "map", Value::object([]))],
            ),
        ),
        (200, completion("The note is still available.", vec![])),
    ]);
    let mut resumed = Agent::new(
        OpenRouter::fixture(resumed_fixture.endpoint.clone()),
        Tools::new(project.path()).unwrap(),
    );
    resumed.enable_sessions(home.path()).unwrap();
    resumed.resume(&id).unwrap();
    resumed
        .run_turn("Show the map again", &mut |_| Ok(()))
        .unwrap();
    let replay = resumed_fixture.finish();
    let overview = receipt(
        replay[1].body.get("messages").unwrap().as_array().unwrap(),
        "resumed-map",
    );
    assert_eq!(
        note(&overview, &note_id)
            .get("freshness")
            .and_then(Value::as_str),
        Some("changed")
    );

    let fresh_fixture = HttpFixture::new(vec![
        (
            200,
            completion(
                "Inspecting a new conversation.",
                vec![tool_call("fresh-map", "map", Value::object([]))],
            ),
        ),
        (200, completion("The project note is shared.", vec![])),
    ]);
    let mut fresh = Agent::new(
        OpenRouter::fixture(fresh_fixture.endpoint.clone()),
        Tools::new(project.path()).unwrap(),
    );
    fresh.enable_sessions(home.path()).unwrap();
    fresh
        .run_turn("Show this project's map", &mut |_| Ok(()))
        .unwrap();
    let fresh_requests = fresh_fixture.finish();
    let shared = receipt(
        fresh_requests[1]
            .body
            .get("messages")
            .unwrap()
            .as_array()
            .unwrap(),
        "fresh-map",
    );
    assert_eq!(
        note(&shared, &note_id)
            .get("freshness")
            .and_then(Value::as_str),
        Some("changed")
    );
}

fn completed(output: Vec<Value>) -> Response {
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

fn openai_call(id: &str, arguments: Value) -> Value {
    Value::object([
        ("type", Value::string("function_call")),
        ("namespace", Value::string("jecode")),
        ("call_id", Value::string(id)),
        ("name", Value::string("map")),
        ("arguments", Value::string(arguments.encode())),
    ])
}

fn output(request: &Value, id: &str) -> Value {
    let item = request
        .get("input")
        .unwrap()
        .as_array()
        .unwrap()
        .iter()
        .find(|item| {
            item.get("call_id").and_then(Value::as_str) == Some(id)
                && item.get("type").and_then(Value::as_str) == Some("function_call_output")
        })
        .unwrap();
    json::parse(item.get("output").and_then(Value::as_str).unwrap()).unwrap()
}

#[test]
fn openai_namespace_map_note_and_overview_reach_followup_requests() {
    let project = Directory::new();
    let home = Directory::new();
    fs::create_dir(project.path().join("src")).unwrap();
    fs::write(project.path().join("src/main.rs"), "fn run() {}\n").unwrap();
    let answer = Value::object([
        ("type", Value::string("message")),
        ("role", Value::string("assistant")),
        ("phase", Value::string("final_answer")),
        (
            "content",
            Value::Array(vec![Value::object([
                ("type", Value::string("output_text")),
                ("text", Value::string("The map is available.")),
            ])]),
        ),
    ]);
    let fixture = HttpFixture::streaming(vec![
        completed(vec![openai_call("openai-remember", remember())]),
        completed(vec![openai_call("openai-overview", Value::object([]))]),
        completed(vec![answer]),
    ]);
    let api = Api::fixture(
        Session::fixture("isolated-access-token"),
        fixture.endpoint.trim_end_matches("/chat/completions"),
    );
    let backend = OpenAi::with_api(api, "fixture-model".into()).unwrap();
    let mut agent = Agent::with_backend(Box::new(backend), Tools::new(project.path()).unwrap());
    agent.enable_sessions(home.path()).unwrap();
    agent
        .run_turn("Record and inspect the project map", &mut |_| Ok(()))
        .unwrap();
    let requests = fixture.finish();
    assert_eq!(requests.len(), 3);
    let namespace = &requests[0].body.get("tools").unwrap().as_array().unwrap()[0];
    let catalog = namespace.get("tools").unwrap().as_array().unwrap();
    assert_eq!(
        catalog
            .iter()
            .rev()
            .nth(1)
            .unwrap()
            .get("name")
            .and_then(Value::as_str),
        Some("map")
    );
    let saved = output(&requests[1].body, "openai-remember");
    let note_id = saved.get("id").and_then(Value::as_str).unwrap();
    let overview = output(&requests[2].body, "openai-overview");
    let entry = note(&overview, note_id);
    assert_eq!(
        entry.get("freshness").and_then(Value::as_str),
        Some("current")
    );
    assert_eq!(
        entry.get("title").and_then(Value::as_str),
        Some("Entry point")
    );
    assert_eq!(
        receipt(&agent.messages.lock().unwrap(), "openai-overview"),
        overview
    );
}
