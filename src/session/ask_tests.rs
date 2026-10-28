use super::*;
use crate::{
    backend::openrouter::OpenRouter,
    config::{Settings, Store},
    json::{self, Value},
    test_support::{Directory, HttpFixture, completion, tool_call},
    tools::Tools,
};
use std::collections::VecDeque;
use std::io::{BufReader, Cursor, Read};

fn agent(project: &Directory, endpoint: &str) -> Agent {
    Agent::new(
        OpenRouter::fixture(endpoint.into()),
        Tools::new(project.path()).unwrap(),
    )
}

fn config(home: &Directory) -> SessionConfig {
    SessionConfig {
        store: Store::new(home.path().join("config.json")),
        settings: Settings::new("isolated-fixture-key".into(), "fixture/model".into()).unwrap(),
        bash: crate::tools::find_bash().unwrap(),
    }
}

fn result(agent: &Agent, id: &str) -> Value {
    agent
        .archive()
        .messages
        .lock()
        .unwrap()
        .iter()
        .find(|message| message.get("tool_call_id").and_then(Value::as_str) == Some(id))
        .and_then(|message| message.get("content"))
        .and_then(Value::as_str)
        .and_then(|text| json::parse(text).ok())
        .unwrap()
}

const FORMAT: &str = r#"{"questions":[{"id":"format","title":"Format",
    "question":"Which export format?","options":[{"id":"csv","label":"CSV"}]}]}"#;

/// Input that closes once after each part, as Ctrl+D (Ctrl+Z on Windows)
/// does in a terminal that stays open.
struct Parts(VecDeque<Cursor<Vec<u8>>>);

impl Read for Parts {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        let Some(part) = self.0.front_mut() else {
            return Ok(0);
        };
        let read = part.read(buffer)?;
        if read == 0 {
            self.0.pop_front();
        }
        Ok(read)
    }
}

fn parts(parts: &[&str]) -> BufReader<Parts> {
    BufReader::new(Parts(
        parts
            .iter()
            .map(|part| Cursor::new(part.as_bytes().to_vec()))
            .collect(),
    ))
}

#[test]
fn plain_questions_take_numbers_lists_and_own_words_one_at_a_time() {
    let project = Directory::new();
    let home = Directory::new();
    let questions = json::parse(
        r#"{"questions":[
        {"id":"rows","title":"Rows","question":"Which rows?","context":"Filters are active.",
         "options":[{"id":"visible","label":"Visible rows","description":"Keep filters","recommended":true},
                    {"id":"all","label":"All rows"}]},
        {"id":"formats","title":"Formats","question":"Which formats?","multiple":true,
         "options":[{"id":"csv","label":"CSV"},{"id":"xlsx","label":"XLSX"},{"id":"json","label":"JSON"}]},
        {"id":"delivery","title":"Delivery","question":"Where should exports go?"}]}"#,
    )
    .unwrap();
    let fixture = HttpFixture::new(vec![
        (
            200,
            completion("", vec![tool_call("ask-1", "ask_user", questions)]),
        ),
        (200, completion("Export settings recorded", vec![])),
    ]);
    let mut agent = agent(&project, &fixture.endpoint);
    let mut output = Vec::new();
    chat(
        &mut agent,
        &mut config(&home),
        &mut Cursor::new("Export the report\n\n2\n3, 1\nEmail the lead\n/exit\n"),
        &mut output,
        &mut Vec::new(),
        Vec::new(),
    )
    .unwrap();
    assert_eq!(
        result(&agent, "ask-1").get("answers").map(Value::encode),
        Some(
            r#"[{"choices":["all"],"id":"rows","text":""},{"choices":["csv","json"],"id":"formats","text":""},{"choices":[],"id":"delivery","text":"Email the lead"}]"#
                .into()
        )
    );
    let output = String::from_utf8(output).unwrap();
    for expected in [
        "Question 1 of 3 · Rows\nWhich rows?\nFilters are active.\n1. Visible rows (recommended)\n   Keep filters\n2. All rows\n",
        "Type an answer, or /dismiss to skip.",
        "Question 2 of 3 · Formats",
        "Answer (numbers such as 1,3 or your own words; /dismiss to skip): ",
        "Question 3 of 3 · Delivery\nWhere should exports go?\nAnswer (/dismiss to skip): ",
    ] {
        assert!(output.contains(expected), "{expected}\n{output}");
    }
    assert_eq!(fixture.finish().len(), 2);
}

#[test]
fn closed_input_saves_the_question_cancels_later_calls_and_resumes_in_place() {
    let project = Directory::new();
    let home = Directory::new();
    let write = Value::object([
        ("path", Value::string("must-not-exist.txt")),
        ("content", Value::string("side effect")),
    ]);
    let fixture = HttpFixture::new(vec![
        (
            200,
            completion(
                "",
                vec![
                    tool_call("ask-1", "ask_user", json::parse(FORMAT).unwrap()),
                    tool_call("write-after", "write", write),
                ],
            ),
        ),
        (200, completion("Answer recorded", vec![])),
    ]);
    let mut agent = agent(&project, &fixture.endpoint);
    let mut status = Vec::new();
    chat(
        &mut agent,
        &mut config(&home),
        &mut parts(&["Export the report\n", "1\n/exit\n"]),
        &mut Vec::new(),
        &mut status,
        Vec::new(),
    )
    .unwrap();
    let status = String::from_utf8(status).unwrap();
    assert_eq!(
        status,
        "[waiting for model: fixture/model]\n[tool: ask_user] Format\n[ask_user: Answer received]\n[waiting for model: fixture/model]\n",
        "no error and no later call ran"
    );
    assert_eq!(
        result(&agent, "write-after")
            .get("outcome")
            .and_then(Value::as_str),
        Some("not_executed")
    );
    assert!(!project.path().join("must-not-exist.txt").exists());
    assert_eq!(
        result(&agent, "ask-1").get("answers").map(Value::encode),
        Some(r#"[{"choices":["csv"],"id":"format","text":""}]"#.into())
    );
    assert!(!agent.has_pending_question());
    assert_eq!(fixture.finish().len(), 2);
}

#[test]
fn a_failed_continuation_after_resuming_a_question_keeps_the_chat_open() {
    let project = Directory::new();
    let home = Directory::new();
    let first = HttpFixture::new(vec![(
        200,
        completion(
            "",
            vec![tool_call("ask-1", "ask_user", json::parse(FORMAT).unwrap())],
        ),
    )]);
    let mut original = agent(&project, &first.endpoint);
    chat(
        &mut original,
        &mut config(&home),
        &mut Cursor::new("Export the report\n"),
        &mut Vec::new(),
        &mut Vec::new(),
        Vec::new(),
    )
    .unwrap();
    assert!(original.has_pending_question());
    let id = original.sessions().unwrap().id();
    drop(original);
    assert_eq!(first.finish().len(), 1);

    let failure = Value::object([(
        "error",
        Value::object([
            ("code", Value::number(401)),
            ("message", Value::string("Invalid key")),
        ]),
    )]);
    let next = HttpFixture::new(vec![(401, failure)]);
    let mut resumed = agent(&project, &next.endpoint);
    let mut output = Vec::new();
    let mut status = Vec::new();
    chat_with_resume(
        &mut resumed,
        &mut config(&home),
        &mut Cursor::new("1\n/exit\n"),
        &mut output,
        &mut status,
        Some(id),
    )
    .unwrap();
    assert!(String::from_utf8(status).unwrap().contains("401"));
    assert!(String::from_utf8(output).unwrap().contains("\n> "));
    assert_eq!(
        result(&resumed, "ask-1").get("answers").map(Value::encode),
        Some(r#"[{"choices":["csv"],"id":"format","text":""}]"#.into())
    );
    assert_eq!(next.finish().len(), 1);
}
