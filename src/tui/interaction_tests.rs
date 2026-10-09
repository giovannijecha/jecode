use super::*;
use crate::{
    config::{Settings, Store},
    effort::Effort,
    json::Value,
    openrouter::OpenRouter,
    test_support::{Directory, HttpFixture, Response, completion},
    tools::Tools,
};
use std::{thread, time::Instant};

fn app(directory: &Directory, fixture: &HttpFixture) -> App {
    let settings = Settings::new("isolated-fixture-key".into(), "fixture/model".into()).unwrap();
    let store = Store::new(directory.path().join("config"));
    store.save(&settings).unwrap();
    App::new(
        Agent::new(
            OpenRouter::fixture(fixture.endpoint.clone()),
            Tools::new(directory.path()).unwrap(),
        ),
        SessionConfig {
            store,
            settings,
            bash: crate::tools::find_bash().unwrap(),
        },
        None,
    )
}
fn text(app: &mut App, value: &str) {
    app.input(Decoded::Text(value.into())).unwrap();
}
fn key(app: &mut App, code: u16, modifiers: u8) {
    assert!(
        !app.input(Decoded::Key(terminal::Key {
            code,
            modifiers,
            character: 0
        }))
        .unwrap()
    );
}
fn submit(app: &mut App, value: &str) {
    text(app, value);
    key(app, 13, 0);
}
fn wait(app: &mut App) {
    let started = Instant::now();
    while app.worker.is_some() || app.job.is_some() {
        app.poll().unwrap();
        assert!(started.elapsed() < Duration::from_secs(6));
        thread::sleep(Duration::from_millis(5));
    }
}
pub(super) fn catalog() -> Value {
    Value::object([(
        "data",
        Value::Array(
            (0..12)
                .map(|i| {
                    Value::object([
                        (
                            "id",
                            Value::string(if i == 0 {
                                "fixture/model".into()
                            } else {
                                format!("fixture/model-{i}")
                            }),
                        ),
                        ("name", Value::string(format!("Fixture model {i}"))),
                        (
                            "supported_parameters",
                            Value::Array(vec![Value::string("tools")]),
                        ),
                        (
                            "reasoning",
                            Value::object([(
                                "supported_efforts",
                                Value::Array(vec![Value::string("high"), Value::string("low")]),
                            )]),
                        ),
                    ])
                })
                .collect(),
        ),
    )])
}

#[test]
fn model_and_effort_commit_together_preserve_context_and_leave_defaults_unchanged() {
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![
        (200, completion("Before.", vec![])),
        (200, catalog()),
        (200, completion("After.", vec![])),
    ]);
    let mut app = app(&directory, &fixture);
    submit(&mut app, "First request");
    wait(&mut app);
    text(&mut app, "unsent draft");
    app.state.editor.cursor = 3;
    app.dispatch("/model".into()).unwrap();
    wait(&mut app);
    assert_eq!(app.state.selector.as_ref().unwrap().options.len(), 12);
    text(&mut app, "model-11");
    key(&mut app, 13, 0);
    assert_eq!(app.archive.model, "fixture/model");
    assert!(matches!(
        app.state.selector.as_ref().unwrap().purpose,
        selector::Purpose::Efforts { .. }
    ));
    text(&mut app, "2");
    assert_eq!(app.archive.model, "fixture/model-11");
    assert_eq!(app.state.effort, "high");
    assert_eq!(app.state.editor.text, "unsent draft");
    assert_eq!(app.state.editor.cursor, 3);
    assert_eq!(
        app.config.store.load().unwrap().unwrap().model,
        "fixture/model"
    );
    app.state.editor.take();
    app.edited();
    submit(&mut app, "Second request");
    wait(&mut app);
    let requests = fixture.finish();
    let messages = requests[2]
        .body
        .get("messages")
        .unwrap()
        .as_array()
        .unwrap();
    assert_eq!(messages.len(), 4);
    assert_eq!(
        messages[2].get("content").and_then(Value::as_str),
        Some("Before.")
    );
    assert!(messages[2].get("reasoning_details").is_none());
    assert_eq!(
        requests[2]
            .body
            .get("reasoning")
            .unwrap()
            .get("effort")
            .and_then(Value::as_str),
        Some("high")
    );
    let export = app.archive.document();
    assert!(export.encode().contains("fixture-reasoning"));
    assert!(
        export
            .get("events")
            .unwrap()
            .encode()
            .contains("Model set to fixture/model-11")
    );
}

#[test]
fn cancel_and_failed_default_save_keep_the_active_pair_and_draft() {
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![(200, catalog()), (200, catalog())]);
    let mut app = app(&directory, &fixture);
    text(&mut app, "draft");
    app.dispatch("/model fixture/model-1".into()).unwrap();
    wait(&mut app);
    key(&mut app, 27, 0);
    assert_eq!(app.archive.model, "fixture/model");
    assert_eq!(app.state.editor.text, "draft");
    let path = app.config.store.path().to_path_buf();
    let original_permissions = std::fs::metadata(&path).unwrap().permissions();
    let mut permissions = original_permissions.clone();
    permissions.set_readonly(true);
    std::fs::set_permissions(&path, permissions).unwrap();
    app.dispatch("/settings".into()).unwrap();
    text(&mut app, "1");
    wait(&mut app);
    text(&mut app, "model-2");
    key(&mut app, 13, 0);
    text(&mut app, "2");
    assert_eq!(app.config.settings.model, "fixture/model");
    assert_eq!(app.archive.model, "fixture/model");
    assert!(matches!(
        app.state.selector.as_ref().unwrap().purpose,
        selector::Purpose::Settings
    ));
    assert!(
        app.state
            .notice
            .as_ref()
            .unwrap()
            .text
            .contains("read-only")
    );
    assert!(
        super::view::frame(&app.state, "fixture/model", "folder")
            .live
            .iter()
            .any(|row| row.plain().contains("read-only"))
    );
    assert_eq!(app.state.editor.text, "draft");
    std::fs::set_permissions(&path, original_permissions).unwrap();
    fixture.finish();
}

#[test]
fn slash_menu_completion_dismissal_and_unknown_commands_are_local() {
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![]);
    let mut app = app(&directory, &fixture);
    text(&mut app, "/MO");
    assert!(app.state.suggestions.visible);
    assert!(app.state.suggestions.panel);
    key(&mut app, 9, 0);
    assert_eq!(app.state.editor.text, "/model");
    key(&mut app, 27, 0);
    assert!(!app.state.suggestions.visible);
    assert!(!app.state.suggestions.panel);
    assert_eq!(app.state.editor.text, "/model");
    key(&mut app, 8, 0);
    assert!(app.state.suggestions.visible);
    text(&mut app, " name");
    assert!(!app.state.suggestions.visible);
    assert!(app.state.suggestions.panel);
    key(&mut app, 27, 0);
    assert!(!app.state.suggestions.panel);
    assert_eq!(app.state.editor.text, "/mode name");
    app.state.editor.take();
    submit(&mut app, "/modle");
    assert!(
        super::view::frame(&app.state, "fixture/model", "folder")
            .live
            .iter()
            .any(|row| row.plain().contains("Did you mean /model"))
    );
    assert_eq!(app.archive.messages.lock().unwrap().len(), 1);
    fixture.finish();
}

#[test]
fn wrapped_command_arguments_follow_the_panel_width_and_keep_the_caret_after_escape() {
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![]);
    let mut app = app(&directory, &fixture);
    app.state.width = 24;
    text(&mut app, "/resume abcdefghijklmnopqrstuvwxyz");
    let before = super::view::frame(&app.state, "fixture/model", "folder");
    let (row, column) = before.cursor.unwrap();
    key(&mut app, 38, 0);
    let moved = super::view::frame(&app.state, "fixture/model", "folder");
    assert_eq!(moved.cursor.unwrap(), (row - 1, column));
    let draft = app.state.editor.clone();
    key(&mut app, 27, 0);
    assert!(!app.state.suggestions.panel);
    assert_eq!(app.state.editor, draft);
    let normal = super::view::frame(&app.state, "fixture/model", "folder");
    assert!(normal.live.iter().any(|line| line.plain().starts_with('─')));
    fixture.finish();
}

#[test]
fn queued_prompts_and_local_commands_drain_in_order() {
    let directory = Directory::new();
    let first = "data: {\"choices\":[{\"delta\":{\"content\":\"First.\"},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n";
    let fixture = HttpFixture::streaming(vec![
        Response::Stream(vec![(Duration::from_millis(250), first.into())]),
        Response::Json(200, completion("Second.", vec![])),
        Response::Json(200, completion("Third.", vec![])),
    ]);
    let mut app = app(&directory, &fixture);
    submit(&mut app, "first");
    submit(&mut app, "/help");
    submit(&mut app, "second\nline");
    submit(&mut app, "/export");
    submit(&mut app, "third");
    assert_eq!(app.state.queue.messages.len(), 4);
    assert!(app.state.editor.text.is_empty());
    wait(&mut app);
    assert!(app.state.information.is_some());
    assert_eq!(app.state.queue.messages.len(), 3);
    key(&mut app, 27, 0);
    app.poll().unwrap();
    wait(&mut app);
    let requests = fixture.finish();
    assert_eq!(requests.len(), 3);
    for (request, prompt) in requests.iter().zip(["first", "second\nline", "third"]) {
        let messages = request.body.get("messages").unwrap().as_array().unwrap();
        assert_eq!(
            messages
                .last()
                .unwrap()
                .get("content")
                .and_then(Value::as_str),
            Some(prompt)
        );
        assert!(!request.body.encode().contains("\"content\":\"/help\""));
    }
    assert!(app.state.queue.messages.is_empty());
    assert!(
        app.state
            .rows()
            .iter()
            .flatten()
            .all(|row| !row.plain().contains("Commands and controls"))
    );
    assert!(
        std::fs::read_dir(directory.path())
            .unwrap()
            .flatten()
            .any(|entry| entry
                .file_name()
                .to_string_lossy()
                .starts_with("JECODE-SESSION-"))
    );
}

#[test]
fn stopping_pauses_fifo_drafts_and_preserves_the_separate_composer() {
    let directory = Directory::new();
    let fixture = HttpFixture::streaming(vec![Response::Stream(vec![
        (Duration::ZERO, "data: {\"choices\":[{\"delta\":{\"reasoning\":\"fixture\"},\"finish_reason\":null}]}\n\n".into()),
        (Duration::from_millis(400), "data: [DONE]\n\n".into()),
    ])]);
    let mut app = app(&directory, &fixture);
    submit(&mut app, "active");
    let started = Instant::now();
    while app.state.activity.as_ref().unwrap().label != "Thinking" {
        app.poll().unwrap();
        assert!(started.elapsed() < Duration::from_secs(3));
        thread::sleep(Duration::from_millis(5));
    }
    submit(&mut app, "queued one");
    submit(&mut app, "queued two");
    text(&mut app, "prior draft");
    key(&mut app, 38, 1);
    key(&mut app, 40, 0);
    key(&mut app, 13, 0);
    text(&mut app, " revised");
    key(&mut app, 13, 0);
    key(&mut app, 27, 0);
    key(&mut app, 27, 0);
    wait(&mut app);
    assert_eq!(app.state.editor.text, "prior draft");
    assert_eq!(
        app.state
            .queue
            .paused
            .iter()
            .map(|draft| draft.text.as_str())
            .collect::<Vec<_>>(),
        ["queued one", "queued two revised"]
    );
    assert!(app.state.queue.messages.is_empty());
    assert!(app.state.activity.is_none());
    assert!(
        app.archive
            .document()
            .get("events")
            .unwrap()
            .encode()
            .contains("turn_error")
    );
    assert_eq!(fixture.finish().len(), 1);
}

#[test]
fn settings_saves_defaults_without_changing_the_current_footer_until_new() {
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![(200, catalog())]);
    let mut app = app(&directory, &fixture);
    app.dispatch("/settings".into()).unwrap();
    text(&mut app, "1");
    wait(&mut app);
    text(&mut app, "model-3");
    key(&mut app, 13, 0);
    text(&mut app, "2");
    assert_eq!(app.archive.model, "fixture/model");
    assert_eq!(app.state.effort, "default");
    let settings = app.config.store.load().unwrap().unwrap();
    assert_eq!(settings.model, "fixture/model-3");
    assert_eq!(settings.effort, Effort::High);
    assert!(matches!(
        app.state.selector.as_ref().unwrap().purpose,
        selector::Purpose::Settings
    ));
    key(&mut app, 27, 0);
    submit(&mut app, "/new");
    assert_eq!(app.archive.model, "fixture/model-3");
    assert_eq!(app.state.effort, "high");
    assert_eq!(app.archive.messages.lock().unwrap().len(), 1);
    fixture.finish();
}
