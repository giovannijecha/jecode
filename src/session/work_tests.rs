use super::*;
use crate::{
    backend::openrouter::OpenRouter,
    config::{Settings, Store},
    json::{self, Value},
    test_support::{Directory, HttpFixture, completion, tool_call},
    tools::Tools,
};
use std::io::Cursor;

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

fn tool_result(agent: &Agent, id: &str) -> Value {
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

#[test]
fn plain_question_accepts_explicit_choice_free_text_and_dismissal() {
    for (answer, outcome, answers) in [
        (
            "1",
            "answered",
            Some(r#"[{"choices":["csv"],"id":"format","text":""}]"#),
        ),
        (
            "Use JSON",
            "answered",
            Some(r#"[{"choices":[],"id":"format","text":"Use JSON"}]"#),
        ),
        ("/dismiss", "dismissed", None),
    ] {
        let project = Directory::new();
        let home = Directory::new();
        let question = json::parse(
            r#"{"questions":[{"id":"format","title":"Format","question":"Which export format?",
            "options":[{"id":"csv","label":"CSV","recommended":true}]}]}"#,
        )
        .unwrap();
        let fixture = HttpFixture::new(vec![
            (
                200,
                completion("", vec![tool_call("ask-1", "ask_user", question)]),
            ),
            (200, completion("Decision received", vec![])),
        ]);
        let mut agent = agent(&project, &fixture.endpoint);
        let mut output = Vec::new();
        chat(
            &mut agent,
            &mut config(&home),
            &mut Cursor::new(format!("Export the report\n{answer}\n/exit\n")),
            &mut output,
            &mut Vec::new(),
            Vec::new(),
        )
        .unwrap();
        let result = tool_result(&agent, "ask-1");
        assert_eq!(result.get("outcome").and_then(Value::as_str), Some(outcome));
        assert_eq!(result.get("answers").map(Value::encode).as_deref(), answers);
        assert!(
            String::from_utf8(output)
                .unwrap()
                .contains("Which export format?")
        );
        assert_eq!(fixture.finish().len(), 2);
    }
}

#[test]
fn plain_todo_reads_persisted_plan_without_a_model_request() {
    let project = Directory::new();
    let home = Directory::new();
    let plan = Value::object([
        ("action", Value::string("set")),
        ("title", Value::string("CSV export")),
        (
            "tasks",
            Value::Array(vec![Value::object([
                ("id", Value::string("t1")),
                ("title", Value::string("Inspect report")),
                ("status", Value::string("done")),
                ("note", Value::string("Filters verified.")),
            ])]),
        ),
    ]);
    let fixture = HttpFixture::new(vec![
        (
            200,
            completion("", vec![tool_call("todo-1", "todo_list", plan)]),
        ),
        (200, completion("The export plan is recorded.", vec![])),
    ]);
    let mut first = agent(&project, &fixture.endpoint);
    let mut output = Vec::new();
    chat(
        &mut first,
        &mut config(&home),
        &mut Cursor::new("Implement CSV export\n/todo\n/exit\n"),
        &mut output,
        &mut Vec::new(),
        Vec::new(),
    )
    .unwrap();
    let output = String::from_utf8(output).unwrap();
    assert!(output.contains("Todo · CSV export · 1 of 1 done\n 1  Inspect report  done"));
    assert!(
        !output.contains("Filters verified."),
        "notes stay for the model"
    );
    let id = first.sessions().unwrap().id();
    drop(first);
    let mut resumed = agent(&project, &fixture.endpoint);
    let mut output = Vec::new();
    chat_with_resume(
        &mut resumed,
        &mut config(&home),
        &mut Cursor::new("/todo\n/exit\n"),
        &mut output,
        &mut Vec::new(),
        Some(id),
    )
    .unwrap();
    assert_eq!(resumed.todo_plan().unwrap().revision, 1);
    assert!(String::from_utf8(output).unwrap().contains("CSV export"));
    assert_eq!(fixture.finish().len(), 2);
}
