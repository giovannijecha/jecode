use super::*;
use crate::{
    config::{Settings, Store},
    json::Value,
    openrouter::OpenRouter,
    test_support::{Directory, HttpFixture, completion},
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
fn enter(app: &mut App) {
    app.input(Decoded::Key(terminal::Key {
        code: 13,
        modifiers: 0,
        character: 13,
    }))
    .unwrap();
}
fn wait(app: &mut App) {
    let started = Instant::now();
    while app.job.is_some() || app.worker.is_some() {
        app.poll().unwrap();
        assert!(started.elapsed() < Duration::from_secs(5));
        thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn validated_key_save_changes_authentication_preserves_context_and_never_displays_the_key() {
    let directory = Directory::new();
    let key = "replacement-fixture-key";
    let fixture = HttpFixture::new(vec![
        (200, Value::object([("data", Value::object([]))])),
        (
            200,
            completion(
                "Echo isolated-fixture-key and replacement-fixture-key",
                vec![],
            ),
        ),
    ]);
    let mut app = app(&directory, &fixture);
    app.state.editor.insert("untouched draft");
    app.dispatch("/settings".into()).unwrap();
    app.input(Decoded::Text("3".into())).unwrap();
    app.input(Decoded::Text(key.into())).unwrap();
    let frame = view::frame(&app.state, "fixture/model", "directory");
    assert!(frame.live.iter().all(|line| !line.plain().contains(key)));
    enter(&mut app);
    wait(&mut app);
    assert_eq!(app.state.editor.text, "untouched draft");
    assert_eq!(app.archive.model, "fixture/model");
    assert_eq!(app.config.store.load().unwrap().unwrap().api_key, key);
    assert!(matches!(
        app.state.selector.as_ref().unwrap().purpose,
        selector::Purpose::Settings
    ));
    app.close_selector();
    app.state.editor.take();
    app.input(Decoded::Text("question".into())).unwrap();
    enter(&mut app);
    wait(&mut app);
    let exported = app.archive.document().encode();
    assert!(!exported.contains(key));
    assert!(!exported.contains("isolated-fixture-key"));
    assert!(
        app.state
            .rows()
            .iter()
            .flatten()
            .all(|line| !line.plain().contains(key))
    );
    let requests = fixture.finish();
    assert!(requests.iter().all(|request| {
        request
            .headers
            .iter()
            .any(|header| header == "Authorization: Bearer replacement-fixture-key")
    }));
}

#[test]
fn a_rejected_key_does_not_replace_saved_settings_or_the_current_client() {
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![(
        401,
        Value::object([(
            "error",
            Value::object([("message", Value::string("Rejected replacement-fixture-key"))]),
        )]),
    )]);
    let mut app = app(&directory, &fixture);
    app.dispatch("/settings".into()).unwrap();
    app.input(Decoded::Text("3".into())).unwrap();
    app.input(Decoded::Text("replacement-fixture-key".into()))
        .unwrap();
    enter(&mut app);
    wait(&mut app);
    assert_eq!(
        app.config.store.load().unwrap().unwrap().api_key,
        "isolated-fixture-key"
    );
    assert!(matches!(
        app.state.selector.as_ref().unwrap().purpose,
        selector::Purpose::Settings
    ));
    assert!(app.state.notice.as_ref().unwrap().text.contains("HTTP 401"));
    assert!(
        app.state
            .rows()
            .iter()
            .flatten()
            .all(|line| !line.plain().contains("replacement-fixture-key"))
    );
    assert!(
        super::view::frame(&app.state, "fixture/model", "folder")
            .live
            .iter()
            .any(|line| line.plain().contains("HTTP 401"))
    );
    fixture.finish();
}

#[test]
fn a_missing_tool_finish_event_is_reconciled_from_retained_protocol_results() {
    use crate::test_support::tool_call;
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![
        (
            200,
            completion(
                "",
                vec![tool_call(
                    "read-1",
                    "read",
                    Value::object([("path", Value::string("missing"))]),
                )],
            ),
        ),
        (200, completion("Done", vec![])),
    ]);
    let mut app = app(&directory, &fixture);
    let mut agent = app.agent.take().unwrap();
    agent
        .run_turn("read missing file", &mut |event| {
            if !matches!(event, crate::events::Event::ToolFinished { .. }) {
                app.state.event(event);
            }
            Ok(())
        })
        .unwrap();
    app.state.settle_tools(&agent.archive());
    app.state.close_tools();
    assert!(
        app.state
            .rows()
            .iter()
            .flatten()
            .any(|line| line.plain().contains("read  missing") && line.plain().contains("× failed"))
    );
    assert!(
        view::frame(&app.state, "fixture", "directory")
            .live
            .iter()
            .all(|line| !line.plain().contains("running"))
    );
    app.agent = Some(agent);
    fixture.finish();
}

#[test]
fn settings_remains_open_for_multiple_default_changes_with_updated_values_and_feedback() {
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![
        (200, interaction_tests::catalog()),
        (200, interaction_tests::catalog()),
    ]);
    let mut app = app(&directory, &fixture);
    app.state.editor.insert("kept draft");
    app.dispatch("/settings".into()).unwrap();
    app.input(Decoded::Text("1".into())).unwrap();
    wait(&mut app);
    app.input(Decoded::Text("model-3".into())).unwrap();
    enter(&mut app);
    app.input(Decoded::Text("2".into())).unwrap();
    assert!(matches!(
        app.state.selector.as_ref().map(|menu| &menu.purpose),
        Some(selector::Purpose::Settings)
    ));
    assert_eq!(app.config.settings.model, "fixture/model-3");
    assert_eq!(app.config.settings.effort, crate::effort::Effort::High);
    assert!(
        app.state
            .notice
            .as_ref()
            .unwrap()
            .text
            .contains("Defaults saved")
    );
    app.input(Decoded::Text("2".into())).unwrap();
    wait(&mut app);
    app.input(Decoded::Text("1".into())).unwrap();
    let menu = app.state.selector.as_ref().unwrap();
    assert!(matches!(menu.purpose, selector::Purpose::Settings));
    assert_eq!(menu.options[0].description, "fixture/model-3");
    assert_eq!(menu.options[1].description, "default");
    assert!(
        app.state
            .notice
            .as_ref()
            .unwrap()
            .text
            .contains("Defaults saved")
    );
    assert_eq!(app.archive.model, "fixture/model");
    assert_eq!(app.state.editor.text, "kept draft");
    assert_eq!(fixture.finish().len(), 2);
}

#[test]
fn cancelling_settings_children_returns_to_settings_before_closing_the_manager() {
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![]);
    let mut app = app(&directory, &fixture);
    app.state.editor.insert("kept draft");
    app.dispatch("/settings".into()).unwrap();
    app.input(Decoded::Text("1".into())).unwrap();
    assert!(app.job.is_some());
    app.close_selector();
    assert!(app.job.is_none());
    assert!(matches!(
        app.state.selector.as_ref().unwrap().purpose,
        selector::Purpose::Settings
    ));
    assert_eq!(app.state.selector.as_ref().unwrap().selected, 0);
    app.input(Decoded::Text("3".into())).unwrap();
    app.input(Decoded::Text("unsaved-fixture-key".into()))
        .unwrap();
    app.close_selector();
    assert!(matches!(
        app.state.selector.as_ref().unwrap().purpose,
        selector::Purpose::Settings
    ));
    assert_eq!(app.state.selector.as_ref().unwrap().selected, 2);
    let displayed = view::frame(&app.state, "fixture/model", "fixture")
        .live
        .iter()
        .map(|line| line.plain())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(!displayed.contains("unsaved-fixture-key"));
    app.close_selector();
    assert!(app.state.selector.is_none());
    assert_eq!(app.state.editor.text, "kept draft");
    assert_eq!(app.config.settings.api_key, "isolated-fixture-key");
    assert!(fixture.finish().is_empty());
}

#[test]
fn catalog_errors_return_to_settings_but_standalone_model_selection_closes() {
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![
        (
            500,
            Value::object([("error", Value::string("Fixture catalog unavailable"))]),
        ),
        (200, interaction_tests::catalog()),
    ]);
    let mut app = app(&directory, &fixture);
    app.dispatch("/settings".into()).unwrap();
    app.input(Decoded::Text("1".into())).unwrap();
    wait(&mut app);
    assert!(matches!(
        app.state.selector.as_ref().unwrap().purpose,
        selector::Purpose::Settings
    ));
    assert!(app.state.notice.as_ref().unwrap().text.contains("HTTP 500"));
    app.close_selector();
    app.dispatch("/model fixture/model-1".into()).unwrap();
    wait(&mut app);
    app.input(Decoded::Text("1".into())).unwrap();
    assert!(app.state.selector.is_none());
    assert_eq!(app.archive.model, "fixture/model-1");
    assert_eq!(fixture.finish().len(), 2);
}
