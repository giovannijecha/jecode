use super::*;
use crate::{
    backend::openrouter::OpenRouter,
    config::{Settings, Store},
    effort::Effort,
    json::Value,
    test_support::{Directory, HttpFixture, completion},
    tools::Tools,
};
use std::{thread, time::Instant};

#[test]
fn a_newly_configured_default_provider_is_available_to_new_conversations() {
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![]);
    let mut app = app(&directory, &fixture);
    let home = app.config.store.path().parent().unwrap().to_path_buf();
    crate::auth::openai::Store::fixture_account(home.clone(), "isolated-new-account-access")
        .unwrap();
    let session = crate::auth::openai::Store::new(home)
        .active()
        .unwrap()
        .unwrap();
    app.catalog_services = Some(
        crate::backend::Services::for_credential(&crate::auth::Credential::openai(session))
            .unwrap(),
    );
    app.apply_selection("fixture-chatgpt-default".into(), Effort::Default, true);
    assert_eq!(app.agent.as_ref().unwrap().backend_id(), "openrouter");
    app.dispatch("/new".into()).unwrap();
    assert_eq!(app.agent.as_ref().unwrap().backend_id(), "openai");
    assert_eq!(app.archive.model, "fixture-chatgpt-default");
    assert!(fixture.finish().is_empty());
}

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
fn open_key(app: &mut App) {
    app.input(Decoded::Text("5".into())).unwrap();
    app.input(Decoded::Text("2".into())).unwrap();
    app.input(Decoded::Text("1".into())).unwrap();
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
fn backend_command_switches_model_without_changing_saved_defaults() {
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![]);
    let mut app = app(&directory, &fixture);
    app.dispatch("/backend".into()).unwrap();
    assert!(app.state.items.iter().any(|item| matches!(item, state::Item::Local { result: Some(result), .. } if result.contains("Available backends"))));
    app.dispatch("/backend openrouter fixture/manual".into())
        .unwrap();
    assert_eq!(app.agent.as_ref().unwrap().model(), "fixture/manual");
    assert_eq!(app.config.settings.model, "fixture/model");
    assert_eq!(app.archive.model, "fixture/manual");
    assert!(fixture.finish().is_empty());
}

#[test]
fn backend_command_uses_the_selected_chatgpt_account_without_changing_defaults() {
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![]);
    let mut app = app(&directory, &fixture);
    let home = app.config.store.path().parent().unwrap().to_path_buf();
    crate::auth::openai::Store::fixture_account(home, "fixture-token").unwrap();
    app.dispatch("/backend openai fixture-chatgpt-model".into())
        .unwrap();
    assert_eq!(app.agent.as_ref().unwrap().backend_id(), "openai");
    assert_eq!(app.archive.model, "fixture-chatgpt-model");
    assert_eq!(app.config.settings.backend_id, "openrouter");
    assert!(matches!(app.state.chatgpt_account,
        Some(state::ChatGptAccount::Active(ref email)) if email == "fixture@example.test"));
    assert!(fixture.finish().is_empty());
}

#[test]
fn two_configured_providers_choose_provider_then_model_and_effort_for_this_conversation() {
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![(
        200,
        Value::object([(
            "models",
            Value::Array(vec![Value::object([
                ("slug", Value::string("fixture-chatgpt-model")),
                ("display_name", Value::string("Fixture ChatGPT model")),
                ("visibility", Value::string("list")),
                ("context_window", Value::number(500_000)),
                (
                    "supported_reasoning_levels",
                    Value::Array(
                        ["low", "medium", "high", "xhigh", "max", "ultra"]
                            .into_iter()
                            .map(|effort| Value::object([("effort", Value::string(effort))]))
                            .collect(),
                    ),
                ),
            ])]),
        )]),
    )]);
    let store = Store::new(directory.path().join("config"));
    let settings =
        Settings::new("isolated-fixture-key".into(), "fixture/router-model".into()).unwrap();
    store.save(&settings).unwrap();
    let home = store.path().parent().unwrap().to_path_buf();
    crate::auth::openai::Store::fixture_account(home.clone(), "fixture-chatgpt-token").unwrap();
    let session = crate::auth::openai::Store::new(home)
        .active()
        .unwrap()
        .unwrap();
    let services = crate::backend::Services::from_service(crate::backend::openai::Api::fixture(
        session,
        fixture.endpoint.strip_suffix("/chat/completions").unwrap(),
    ));
    let agent = Agent::with_backend(
        services.create("fixture/original".into()).unwrap(),
        Tools::new(directory.path()).unwrap(),
    );
    let mut app = App::new(
        agent,
        SessionConfig {
            store,
            settings,
            bash: crate::tools::find_bash().unwrap(),
        },
        None,
    );
    app.dispatch("/model".into()).unwrap();
    let menu = app.state.selector.as_ref().unwrap();
    assert!(matches!(
        menu.purpose,
        selector::Purpose::Providers { defaults: false }
    ));
    let index = menu
        .options
        .iter()
        .position(|row| matches!(row.choice, selector::Choice::Provider("openai")))
        .unwrap();
    assert!(menu.options[index].prose.contains("fixture@example.test"));
    app.choose(selector::Choice::Provider("openai")).unwrap();
    assert!(index < 2);
    assert!(app.job.is_some());
    wait(&mut app);
    assert!(matches!(
        app.state.selector.as_ref().unwrap().purpose,
        selector::Purpose::Models { defaults: false }
    ));
    assert!(
        app.state.selector.as_ref().unwrap().options[0]
            .prose
            .contains("fixture@example.test")
    );
    app.state.width = 44;
    assert!(
        view::frame(&app.state, "fixture/original", "directory")
            .live
            .iter()
            .any(|row| row.plain().contains("fixture@example.test"))
    );
    enter(&mut app);
    assert!(matches!(
        app.state.selector.as_ref().unwrap().purpose,
        selector::Purpose::Efforts {
            defaults: false,
            ..
        }
    ));
    assert!(
        app.state
            .selector
            .as_ref()
            .unwrap()
            .options
            .iter()
            .any(|row| matches!(row.choice, selector::Choice::Effort(Effort::Ultra)))
    );
    app.input(Decoded::Text("4".into())).unwrap();
    assert_eq!(app.agent.as_ref().unwrap().backend_id(), "openai");
    assert_eq!(app.agent.as_ref().unwrap().effort(), Effort::High);
    assert_eq!(
        app.agent.as_ref().unwrap().context_status().capacity,
        Some(500_000)
    );
    assert_eq!(app.archive.model, "fixture-chatgpt-model");
    assert_eq!(app.config.settings.backend_id, "openrouter");
    assert!(
        fixture
            .finish()
            .iter()
            .any(|request| request.headers[0].starts_with("GET /models "))
    );
}

#[test]
fn model_command_reports_when_no_provider_is_configured() {
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![]);
    let mut app = app(&directory, &fixture);
    app.config.settings.credentials.clear();
    app.config.settings.api_key.clear();
    app.dispatch("/model".into()).unwrap();
    assert!(app.job.is_none());
    assert!(app.state.selector.is_none());
    assert!(
        app.state
            .notice
            .as_ref()
            .unwrap()
            .text
            .contains("No configured provider")
    );
    assert!(fixture.finish().is_empty());
}

#[test]
fn account_menu_keeps_sign_out_confirmation_and_escape_returns_to_account() {
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![]);
    let mut app = app(&directory, &fixture);
    let home = app.config.store.path().parent().unwrap().to_path_buf();
    crate::auth::openai::Store::fixture_account(home, "fixture-token").unwrap();
    app.dispatch("/settings".into()).unwrap();
    for number in ["5", "1", "1", "1", "3"] {
        app.input(Decoded::Text(number.into())).unwrap();
    }
    assert!(matches!(
        app.state.selector.as_ref().unwrap().purpose,
        selector::Purpose::Account {
            confirm_sign_out: true
        }
    ));
    app.close_selector();
    assert!(matches!(
        app.state.selector.as_ref().unwrap().purpose,
        selector::Purpose::Account {
            confirm_sign_out: false
        }
    ));
    assert!(fixture.finish().is_empty());
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
    open_key(&mut app);
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
    open_key(&mut app);
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
    let rows = app.state.rows();
    let tool = rows
        .iter()
        .find(|block| {
            block
                .iter()
                .any(|line| line.plain().contains("Read missing"))
        })
        .unwrap();
    assert!(tool.iter().any(|line| line.plain().contains("● Activity")));
    assert!(tool.iter().any(|line| line.plain().contains("Not found")));
    assert!(app.state.items.iter().any(|item| matches!(item,
        state::Item::Tool { result: Some(result), .. }
            if result.get("error").and_then(Value::as_str).is_some())));
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
            .contains("Default model:")
    );
    app.input(Decoded::Text("2".into())).unwrap();
    wait(&mut app);
    app.input(Decoded::Text("1".into())).unwrap();
    let menu = app.state.selector.as_ref().unwrap();
    assert!(matches!(menu.purpose, selector::Purpose::Settings));
    assert_eq!(menu.options[0].fact, "fixture/model-3");
    assert_eq!(menu.options[1].fact, "default");
    assert!(
        app.state
            .notice
            .as_ref()
            .unwrap()
            .text
            .contains("Default model:")
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
    open_key(&mut app);
    app.input(Decoded::Text("unsaved-fixture-key".into()))
        .unwrap();
    app.close_selector();
    app.close_selector();
    app.close_selector();
    assert!(matches!(
        app.state.selector.as_ref().unwrap().purpose,
        selector::Purpose::Settings
    ));
    assert_eq!(app.state.selector.as_ref().unwrap().selected, 4);
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
