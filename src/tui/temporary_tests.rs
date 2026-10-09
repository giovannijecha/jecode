use super::*;
use crate::{
    json::Value,
    openrouter::OpenRouter,
    test_support::{Directory, HttpFixture, completion, tool_call},
    tools::Tools,
};
use std::{fs, thread};

fn app(home: &Directory, project: &Directory, endpoint: &str) -> App {
    let mut agent = Agent::new(
        OpenRouter::fixture(endpoint.into()),
        Tools::new(project.path()).unwrap(),
    );
    agent.enable_sessions(home.path()).unwrap();
    App::new(agent, tests::config(project), None)
}

fn submit(app: &mut App, text: &str) {
    app.state.editor.replace(text.into());
    app.edited();
    assert!(!app.submit().unwrap());
}

#[test]
fn temporary_cleanup_queues_until_tools_finish_and_keeps_the_draft_and_quiet_confirmation() {
    let home = Directory::new();
    let project = Directory::new();
    let fixture = HttpFixture::new(vec![
        (
            200,
            completion(
                "",
                vec![
                    tool_call(
                        "probe",
                        "write",
                        Value::object([
                            ("path", Value::string("tmp:probe.txt")),
                            ("content", Value::string("scratch")),
                        ]),
                    ),
                    tool_call(
                        "source",
                        "write",
                        Value::object([
                            ("path", Value::string("source.txt")),
                            ("content", Value::string("source")),
                        ]),
                    ),
                ],
            ),
        ),
        (200, completion("Finished the probe", vec![])),
    ]);
    let mut app = app(&home, &project, &fixture.endpoint);
    submit(&mut app, "Create a scratch probe and a source file");
    submit(&mut app, "/tmp clean");
    assert_eq!(
        app.state
            .queue
            .messages
            .front()
            .map(|prompt| prompt.text.as_str()),
        Some("/tmp clean")
    );
    app.state.editor.replace("retained\ndraft".into());
    app.state.editor.cursor = 3;
    app.edited();
    let deadline = Instant::now() + Duration::from_secs(8);
    while app.worker.is_some() || !app.state.queue.messages.is_empty() {
        app.poll().unwrap();
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(app.state.editor.text, "retained\ndraft");
    assert_eq!(app.state.editor.cursor, 3);
    assert_eq!(
        app.agent.as_ref().unwrap().temporary_info().unwrap().files,
        0
    );
    assert_eq!(
        fs::read_to_string(project.path().join("source.txt")).unwrap(),
        "source"
    );
    let rows = app
        .state
        .rows()
        .into_iter()
        .flatten()
        .map(|row| row.plain())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(!rows.contains("Cleared 1 temporary files"));
    assert!(!rows.contains("/tmp clean"));
    assert!(rows.contains("Finished the probe"));
    app.dispatch("/tmp".into()).unwrap();
    let rows = super::view::frame(&app.state, "fixture/model", "fixture")
        .live
        .into_iter()
        .map(|row| row.plain())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(rows.contains("Session temporary files"));
    assert!(rows.contains("retention"));
    assert!(!rows.lines().any(|line| line == "/tmp"));
    assert_eq!(app.state.editor.text, "retained\ndraft");
    let saved = app.persistence.as_ref().unwrap().snapshot();
    assert!(
        saved
            .events
            .iter()
            .any(|event| event.get("type").and_then(Value::as_str) == Some("temporary_cleanup"))
    );
    assert!(
        saved
            .events
            .iter()
            .any(|event| event.get("command").and_then(Value::as_str) == Some("/tmp clean"))
    );
    assert_eq!(fixture.finish().len(), 2);
}

#[test]
fn unknown_temporary_arguments_show_an_error_and_preserve_the_area() {
    let home = Directory::new();
    let project = Directory::new();
    let mut app = app(&home, &project, "http://127.0.0.1:1/chat/completions");
    let path = app.agent.as_ref().unwrap().temporary_info().unwrap().path;
    fs::write(std::path::Path::new(&path).join("keep.txt"), "keep").unwrap();
    app.dispatch("/tmp unknown".into()).unwrap();
    assert_eq!(
        app.agent.as_ref().unwrap().temporary_info().unwrap().files,
        1
    );
    let rows = super::view::frame(&app.state, "fixture/model", "fixture")
        .live
        .into_iter()
        .map(|row| row.plain())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(rows.contains("Usage: /tmp [clean]"));
    let matches = crate::session::commands::suggestions("/TM");
    assert_eq!(
        crate::session::commands::COMMANDS[matches[0].index].name,
        "/tmp"
    );
}
