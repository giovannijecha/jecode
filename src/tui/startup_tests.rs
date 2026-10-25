use super::*;
use crate::{
    backend::{Services, unconfigured},
    config::{Settings, Store},
    effort::Effort,
    test_support::Directory,
    tools::Tools,
};

fn app(directory: &Directory, model: &str, services: Option<Services>) -> App {
    let settings = Settings::selected("openai", String::new(), model.into()).unwrap();
    let store = Store::new(directory.path().join("config"));
    store.save(&settings).unwrap();
    let agent = Agent::with_backend(
        unconfigured::client(
            "openai",
            model.into(),
            services,
            "No configured provider".into(),
        ),
        Tools::new(directory.path()).unwrap(),
    );
    App::new(
        agent,
        SessionConfig {
            store,
            settings,
            bash: crate::tools::find_bash().unwrap(),
        },
        None,
    )
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
