use super::*;
use crate::{
    backend::{Services, unconfigured},
    config::{Settings, Store},
    effort::Effort,
    test_support::{Directory, HttpFixture},
    tools::Tools,
};

fn app(directory: &Directory, model: &str, services: Option<Services>) -> App {
    let settings = Settings::selected("openai", String::new(), model.into()).unwrap();
    with_settings(directory, settings, services)
}

fn with_settings(directory: &Directory, settings: Settings, services: Option<Services>) -> App {
    let store = Store::new(directory.path().join("config"));
    if settings.has_model() {
        store.save(&settings).unwrap();
    }
    let agent = Agent::with_backend(
        unconfigured::client(
            &settings.backend_id,
            settings.has_model().then(|| settings.model.clone()),
            services,
            "No configured provider".into(),
        ),
        Tools::new(directory.path()).unwrap(),
    );
    App::new(agent, SessionConfig { store, settings }, None)
}

#[test]
fn missing_credentials_keep_settings_available_and_return_unsent_work() {
    let directory = Directory::new();
    let mut app = app(&directory, "saved-model", None);
    app.state
        .editor
        .insert("Keep this task while I configure a provider");
    app.submit().unwrap();
    tests::finish(&mut app);
    assert_eq!(
        app.state.editor.text,
        "Keep this task while I configure a provider"
    );
    assert!(
        app.state
            .notice
            .as_ref()
            .unwrap()
            .text
            .contains("No configured provider")
    );
    app.dispatch("/settings".into()).unwrap();
    assert!(app.state.selector.is_some());
    app.open_credentials();
    assert!(app.state.selector.is_some());
    app.open_api_keys();
    assert!(app.state.selector.is_some());
    app.config.settings.auto_compact = 70;
    app.config.store.save(&app.config.settings).unwrap();
    assert_eq!(app.config.store.load().unwrap().unwrap().auto_compact, 70);
}

#[test]
fn configuring_the_same_model_replaces_the_unconfigured_connection() {
    let directory = Directory::new();
    let home = directory.path().join("config");
    crate::auth::openai::Store::fixture_account(home.clone(), "isolated-access-token").unwrap();
    let settings = Settings::selected("openai", String::new(), "saved-model".into()).unwrap();
    let services = settings.services(&home, "openai").unwrap();
    let mut app = app(&directory, "saved-model", Some(services.clone()));
    assert!(app.agent.as_ref().unwrap().needs_configuration());
    app.catalog_services = Some(services);
    app.apply_selection("saved-model".into(), Effort::Default, false);
    assert!(!app.agent.as_ref().unwrap().needs_configuration());
    assert_eq!(app.archive.model, "saved-model");
    assert!(app.agent.as_ref().unwrap().services().account().is_some());
}

#[test]
fn adding_a_key_after_removing_chatgpt_requests_an_appropriate_model() {
    let directory = Directory::new();
    let mut app = app(&directory, "saved-model", None);
    let fixture = crate::test_support::HttpFixture::new(vec![(
        200,
        crate::json::Value::object([("data", crate::json::Value::Array(vec![]))]),
    )]);
    app.finish_key(
        "isolated-fixture-key".into(),
        Ok(Services::fixture(&fixture.endpoint)),
    )
    .unwrap();
    assert!(
        app.job.is_some(),
        "Connecting a provider must offer its models"
    );
    tests::wait_until(&mut app, |app| app.job.is_none());
    app.apply_selection("fixture/chosen".into(), Effort::Default, false);
    assert!(!app.agent.as_ref().unwrap().needs_configuration());
    assert_eq!(app.agent.as_ref().unwrap().backend_id(), "openrouter");
    assert_eq!(
        app.config.store.load().unwrap().unwrap().backend_id,
        "openai"
    );
    assert_eq!(fixture.finish().len(), 1);
}

#[test]
fn connecting_chatgpt_after_removing_an_openrouter_key_offers_its_models() {
    let directory = Directory::new();
    let settings = Settings::selected("openrouter", String::new(), "saved-model".into()).unwrap();
    let mut app = with_settings(&directory, settings, None);
    let home = directory.path().join("config");
    crate::auth::openai::Store::fixture_account(home.clone(), "isolated-chatgpt-token").unwrap();
    let session = crate::auth::openai::Store::new(home)
        .active()
        .unwrap()
        .unwrap();
    let fixture = HttpFixture::new(vec![(
        200,
        crate::json::Value::object([("models", crate::json::Value::Array(vec![]))]),
    )]);
    let services = Services::from_service(crate::backend::openai::Api::fixture(
        session,
        &fixture.endpoint,
    ));
    app.offer_connected_models(services);
    assert!(app.job.is_some());
    assert_eq!(app.catalog_services.as_ref().unwrap().id(), "openai");
    tests::wait_until(&mut app, |app| app.job.is_none());
    app.apply_selection("chatgpt-model".into(), Effort::Default, false);
    assert!(!app.agent.as_ref().unwrap().needs_configuration());
    assert_eq!(app.agent.as_ref().unwrap().backend_id(), "openai");
    assert_eq!(fixture.finish().len(), 1);
}

#[test]
fn first_key_is_saved_only_after_a_real_model_is_selected_and_activated() {
    let directory = Directory::new();
    let mut app = with_settings(&directory, Settings::unconfigured(), None);
    let catalog = crate::json::Value::object([("data", crate::json::Value::Array(vec![]))]);
    let fixture = HttpFixture::new(vec![(200, catalog.clone()), (200, catalog)]);
    app.finish_key(
        "isolated-fixture-key".into(),
        Ok(Services::fixture(&fixture.endpoint)),
    )
    .unwrap();
    assert!(!app.config.store.path().exists());
    assert!(app.agent.as_ref().unwrap().needs_configuration());
    tests::wait_until(&mut app, |app| app.job.is_none());
    app.dispatch("/model fixture/chosen".into()).unwrap();
    tests::wait_until(&mut app, |app| app.job.is_none());
    let saved = app.config.store.load().unwrap().unwrap();
    assert_eq!(saved.model, "fixture/chosen");
    assert_eq!(saved.api_key, "isolated-fixture-key");
    assert!(!app.agent.as_ref().unwrap().needs_configuration());
    assert_eq!(app.archive.model, "fixture/chosen");
    assert_eq!(fixture.finish().len(), 2);
}

#[test]
fn first_session_preferences_do_not_create_an_incomplete_configuration() {
    let directory = Directory::new();
    let mut app = with_settings(&directory, Settings::unconfigured(), None);
    app.state.selector = Some(selector::Selector::auto_compact(
        app.config.settings.auto_compact,
    ));
    app.state.selector.as_mut().unwrap().editor.text = "70".into();
    app.save_auto_compact();
    assert_eq!(app.config.settings.auto_compact, 70);
    assert!(!app.config.store.path().exists());
    assert!(
        app.state
            .notice
            .as_ref()
            .unwrap()
            .text
            .contains("this session")
    );
}

#[test]
fn changing_effort_before_selecting_a_model_does_not_save_the_display_placeholder() {
    let directory = Directory::new();
    let mut app = with_settings(&directory, Settings::unconfigured(), None);
    app.dispatch("/effort default".into()).unwrap();
    assert!(!app.config.settings.has_model());
    assert!(!app.config.store.path().exists());
    assert!(
        app.state
            .notice
            .as_ref()
            .unwrap()
            .text
            .contains("Choose a model")
    );
}
