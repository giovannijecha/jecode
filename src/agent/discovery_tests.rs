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

fn catalog_names(tools: &[Value]) -> Vec<&str> {
    tools
        .iter()
        .map(|tool| tool.get("name").and_then(Value::as_str).unwrap())
        .collect()
}

#[test]
fn openrouter_discovery_results_reach_the_next_request_and_survive_resume() {
    let project = Directory::new();
    let home = Directory::new();
    fs::create_dir(project.path().join("src")).unwrap();
    fs::write(
        project.path().join("src/alpha.rs"),
        "first\nneedle here\nlast\n",
    )
    .unwrap();
    let first = HttpFixture::new(vec![
        (
            200,
            completion(
                "Inspecting source files.",
                vec![
                    tool_call(
                        "list-source",
                        "list",
                        Value::object([
                            ("path", Value::string("src")),
                            ("recursive", Value::Bool(true)),
                        ]),
                    ),
                    tool_call(
                        "search-source",
                        "search",
                        Value::object([
                            ("query", Value::string("needle")),
                            ("path", Value::string("src")),
                        ]),
                    ),
                ],
            ),
        ),
        (200, completion("Found the source line.", vec![])),
    ]);
    let mut agent = Agent::new(
        OpenRouter::fixture(first.endpoint.clone()),
        Tools::new(project.path()).unwrap(),
    );
    agent.enable_sessions(home.path()).unwrap();
    agent
        .run_turn("Find the source line", &mut |_| Ok(()))
        .unwrap();
    let id = agent.sessions().unwrap().id();
    let requests = first.finish();
    assert_eq!(requests.len(), 2);
    let catalog = requests[0]
        .body
        .get("tools")
        .unwrap()
        .as_array()
        .unwrap()
        .iter()
        .map(|wrapper| wrapper.get("function").unwrap().clone())
        .collect::<Vec<_>>();
    assert_eq!(&catalog_names(&catalog)[9..], &["list", "search"]);
    let next = requests[1]
        .body
        .get("messages")
        .unwrap()
        .as_array()
        .unwrap();
    let listed = receipt(next, "list-source");
    let entries = listed.get("entries").unwrap().as_array().unwrap();
    assert!(
        entries
            .iter()
            .any(|entry| { entry.get("path").and_then(Value::as_str) == Some("src/alpha.rs") })
    );
    assert_eq!(
        listed.get("entries_returned"),
        Some(&Value::number(entries.len()))
    );
    let searched = receipt(next, "search-source");
    let matches = searched.get("matches").unwrap().as_array().unwrap();
    assert_eq!(matches.len(), 1);
    assert_eq!(
        matches[0].get("path").and_then(Value::as_str),
        Some("src/alpha.rs")
    );
    assert_eq!(matches[0].get("line"), Some(&Value::number(2)));
    assert_eq!(
        matches[0].get("text").and_then(Value::as_str),
        Some("needle here")
    );
    let original = agent.messages.lock().unwrap().clone();
    assert_eq!(receipt(&original, "search-source"), searched);
    drop(agent);

    let second = HttpFixture::new(vec![(
        200,
        completion("The result is still available.", vec![]),
    )]);
    let mut resumed = Agent::new(
        OpenRouter::fixture(second.endpoint.clone()),
        Tools::new(project.path()).unwrap(),
    );
    resumed.enable_sessions(home.path()).unwrap();
    resumed.resume(&id).unwrap();
    assert_eq!(
        receipt(&resumed.messages.lock().unwrap(), "list-source"),
        listed
    );
    assert_eq!(
        receipt(&resumed.messages.lock().unwrap(), "search-source"),
        searched
    );
    resumed
        .run_turn("Use the previous result", &mut |_| Ok(()))
        .unwrap();
    let resumed_requests = second.finish();
    let replay = resumed_requests[0]
        .body
        .get("messages")
        .unwrap()
        .as_array()
        .unwrap();
    assert_eq!(receipt(replay, "search-source"), searched);
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

#[test]
fn openai_namespace_carries_empty_search_receipt_to_the_followup() {
    let project = Directory::new();
    fs::write(project.path().join("note.txt"), "ordinary text\n").unwrap();
    let call = Value::object([
        ("type", Value::string("function_call")),
        ("namespace", Value::string("jecode")),
        ("call_id", Value::string("empty-search")),
        ("name", Value::string("search")),
        ("arguments", Value::string(r#"{"query":"not present"}"#)),
    ]);
    let answer = Value::object([
        ("type", Value::string("message")),
        ("role", Value::string("assistant")),
        ("phase", Value::string("final_answer")),
        (
            "content",
            Value::Array(vec![Value::object([
                ("type", Value::string("output_text")),
                ("text", Value::string("No match.")),
            ])]),
        ),
    ]);
    let fixture = HttpFixture::streaming(vec![completed(vec![call]), completed(vec![answer])]);
    let api = Api::fixture(
        Session::fixture("isolated-access-token"),
        fixture.endpoint.trim_end_matches("/chat/completions"),
    );
    let backend = OpenAi::with_api(api, "fixture-model".into()).unwrap();
    let mut agent = Agent::with_backend(Box::new(backend), Tools::new(project.path()).unwrap());
    agent
        .run_turn("Search for missing text", &mut |_| Ok(()))
        .unwrap();
    let requests = fixture.finish();
    let namespace = &requests[0].body.get("tools").unwrap().as_array().unwrap()[0];
    assert_eq!(
        namespace.get("name").and_then(Value::as_str),
        Some("jecode")
    );
    let catalog = namespace.get("tools").unwrap().as_array().unwrap();
    assert_eq!(&catalog_names(catalog)[9..], &["list", "search"]);
    assert!(
        catalog
            .iter()
            .all(|tool| tool.get("strict") == Some(&Value::Bool(false)))
    );
    let output = requests[1]
        .body
        .get("input")
        .unwrap()
        .as_array()
        .unwrap()
        .iter()
        .find(|item| {
            item.get("call_id").and_then(Value::as_str) == Some("empty-search")
                && item.get("type").and_then(Value::as_str) == Some("function_call_output")
        })
        .unwrap();
    let result = json::parse(output.get("output").and_then(Value::as_str).unwrap()).unwrap();
    assert_eq!(result.get("matches"), Some(&Value::Array(vec![])));
    assert_eq!(result.get("matches_returned"), Some(&Value::number(0)));
    assert_eq!(result.get("complete"), Some(&Value::Bool(true)));
    assert_eq!(
        receipt(&agent.messages.lock().unwrap(), "empty-search"),
        result
    );
}
