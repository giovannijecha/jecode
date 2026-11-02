use super::*;

fn configure_five_providers(app: &mut App) {
    for id in ["openai-api", "anthropic-api"] {
        app.config
            .settings
            .credentials
            .insert(id.into(), format!("isolated-{id}-key"));
    }
    let home = app.config.store.path().parent().unwrap().to_path_buf();
    crate::auth::openai::Store::fixture_account(home.clone(), "fixture-chatgpt-token").unwrap();
    crate::auth::anthropic::Store::fixture_account(home, "fixture-claude-token").unwrap();
    app.config.store.save(&app.config.settings).unwrap();
}

#[test]
fn model_categories_filter_five_providers_and_escape_restores_each_level() {
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![
        (200, interaction_tests::catalog()),
        (200, interaction_tests::catalog()),
    ]);
    let mut app = app(&directory, &fixture);
    configure_five_providers(&mut app);
    app.state.editor.insert("draft stays");
    app.dispatch("/model".into()).unwrap();
    let groups = app.state.selector.as_ref().unwrap();
    assert!(matches!(
        groups.purpose,
        selector::Purpose::ProviderGroups { defaults: false }
    ));
    assert_eq!(
        groups
            .options
            .iter()
            .map(|row| row.name.as_str())
            .collect::<Vec<_>>(),
        ["API keys", "Credentials"]
    );
    assert_eq!(groups.options[0].badge, Some(selector::Badge::Current));
    assert_eq!(groups.selected, 0);
    app.choose(selector::Choice::ProviderGroup(
        selector::ProviderGroup::Credentials,
    ))
    .unwrap();
    let providers = app.state.selector.as_ref().unwrap();
    assert!(matches!(
        providers.purpose,
        selector::Purpose::Providers {
            defaults: false,
            group: selector::ProviderGroup::Credentials
        }
    ));
    assert_eq!(
        providers
            .options
            .iter()
            .map(|row| row.name.as_str())
            .collect::<Vec<_>>(),
        ["OpenAI", "Anthropic"]
    );
    app.close_selector();
    let groups = app.state.selector.as_ref().unwrap();
    assert!(matches!(
        groups.purpose,
        selector::Purpose::ProviderGroups { defaults: false }
    ));
    assert_eq!(groups.selected, 1);

    app.choose(selector::Choice::ProviderGroup(
        selector::ProviderGroup::ApiKeys,
    ))
    .unwrap();
    let providers = app.state.selector.as_ref().unwrap();
    assert_eq!(
        providers
            .options
            .iter()
            .map(|row| row.name.as_str())
            .collect::<Vec<_>>(),
        ["OpenRouter", "OpenAI API", "Anthropic API"]
    );
    assert_eq!(providers.options[0].badge, Some(selector::Badge::Current));
    app.choose(selector::Choice::Provider("openrouter"))
        .unwrap();
    wait(&mut app);
    assert!(matches!(
        app.state.selector.as_ref().unwrap().purpose,
        selector::Purpose::Models { defaults: false }
    ));
    app.close_selector();
    assert!(matches!(
        app.state.selector.as_ref().unwrap().purpose,
        selector::Purpose::Providers {
            defaults: false,
            group: selector::ProviderGroup::ApiKeys
        }
    ));
    app.choose(selector::Choice::Provider("openrouter"))
        .unwrap();
    wait(&mut app);
    app.choose(selector::Choice::Model(1)).unwrap();
    assert!(matches!(
        app.state.selector.as_ref().unwrap().purpose,
        selector::Purpose::Efforts {
            defaults: false,
            ..
        }
    ));
    app.close_selector();
    assert!(matches!(
        app.state.selector.as_ref().unwrap().purpose,
        selector::Purpose::Models { defaults: false }
    ));
    app.choose(selector::Choice::Model(1)).unwrap();
    app.choose(selector::Choice::Effort(Effort::High)).unwrap();
    assert_eq!(app.agent.as_ref().unwrap().model(), "fixture/model-1");
    assert_eq!(app.agent.as_ref().unwrap().effort(), Effort::High);
    assert_eq!(app.config.settings.model, "fixture/model");
    assert_eq!(app.state.editor.text, "draft stays");
    assert_eq!(fixture.finish().len(), 2);
}

#[test]
fn default_model_categories_keep_saved_scope_and_return_to_settings() {
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![
        (200, interaction_tests::catalog()),
        (200, interaction_tests::catalog()),
    ]);
    let mut app = app(&directory, &fixture);
    configure_five_providers(&mut app);
    app.state.editor.insert("default draft");
    app.dispatch("/settings".into()).unwrap();
    app.choose(selector::Choice::Setting(selector::Setting::Model))
        .unwrap();
    let groups = app.state.selector.as_ref().unwrap();
    assert!(matches!(
        groups.purpose,
        selector::Purpose::ProviderGroups { defaults: true }
    ));
    assert_eq!(groups.title, "Default provider");
    assert_eq!(groups.options[0].badge, Some(selector::Badge::Default));
    app.choose(selector::Choice::ProviderGroup(
        selector::ProviderGroup::ApiKeys,
    ))
    .unwrap();
    let providers = app.state.selector.as_ref().unwrap();
    assert!(matches!(
        providers.purpose,
        selector::Purpose::Providers {
            defaults: true,
            group: selector::ProviderGroup::ApiKeys
        }
    ));
    assert_eq!(providers.context, "new conversations");
    app.choose(selector::Choice::Provider("openrouter"))
        .unwrap();
    wait(&mut app);
    assert!(matches!(
        app.state.selector.as_ref().unwrap().purpose,
        selector::Purpose::Models { defaults: true }
    ));
    app.choose(selector::Choice::Model(1)).unwrap();
    assert!(matches!(
        app.state.selector.as_ref().unwrap().purpose,
        selector::Purpose::Efforts { defaults: true, .. }
    ));
    app.close_selector();
    assert!(matches!(
        app.state.selector.as_ref().unwrap().purpose,
        selector::Purpose::Models { defaults: true }
    ));
    app.close_selector();
    assert!(matches!(
        app.state.selector.as_ref().unwrap().purpose,
        selector::Purpose::Providers {
            defaults: true,
            group: selector::ProviderGroup::ApiKeys
        }
    ));
    app.close_selector();
    assert!(matches!(
        app.state.selector.as_ref().unwrap().purpose,
        selector::Purpose::ProviderGroups { defaults: true }
    ));
    assert_eq!(app.state.selector.as_ref().unwrap().selected, 0);
    app.choose(selector::Choice::ProviderGroup(
        selector::ProviderGroup::ApiKeys,
    ))
    .unwrap();
    app.choose(selector::Choice::Provider("openrouter"))
        .unwrap();
    wait(&mut app);
    app.choose(selector::Choice::Model(1)).unwrap();
    app.choose(selector::Choice::Effort(Effort::High)).unwrap();
    assert_eq!(app.config.settings.model, "fixture/model-1");
    assert_eq!(app.config.settings.effort, Effort::High);
    assert_eq!(app.agent.as_ref().unwrap().model(), "fixture/model");
    assert_eq!(app.state.editor.text, "default draft");
    assert!(matches!(
        app.state.selector.as_ref().unwrap().purpose,
        selector::Purpose::Settings
    ));
    app.choose(selector::Choice::Setting(selector::Setting::Model))
        .unwrap();
    app.close_selector();
    assert!(matches!(
        app.state.selector.as_ref().unwrap().purpose,
        selector::Purpose::Settings
    ));
    assert_eq!(fixture.finish().len(), 2);
}

#[test]
fn one_configured_category_still_opens_its_provider_list() {
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![]);
    let mut app = app(&directory, &fixture);
    app.config
        .settings
        .credentials
        .insert("openai-api".into(), "fixture-api-key".into());
    app.dispatch("/model".into()).unwrap();
    let groups = app.state.selector.as_ref().unwrap();
    assert_eq!(groups.options.len(), 1);
    assert!(matches!(
        groups.options[0].choice,
        selector::Choice::ProviderGroup(selector::ProviderGroup::ApiKeys)
    ));
    app.choose(selector::Choice::ProviderGroup(
        selector::ProviderGroup::ApiKeys,
    ))
    .unwrap();
    assert_eq!(app.state.selector.as_ref().unwrap().options.len(), 2);
    app.close_selector();
    assert!(matches!(
        app.state.selector.as_ref().unwrap().purpose,
        selector::Purpose::ProviderGroups { defaults: false }
    ));
    assert!(fixture.finish().is_empty());
}

#[test]
fn openai_api_model_route_shows_supported_efforts_and_applies_high() {
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![(
        200,
        Value::object([(
            "data",
            Value::Array(vec![Value::object([("id", Value::string("gpt-6.1-sol"))])]),
        )]),
    )]);
    let endpoint = fixture.endpoint.strip_suffix("/chat/completions").unwrap();
    let services = crate::backend::Services::from_service(
        crate::backend::openai::Api::fixture_api_key("fixture-api-key", endpoint),
    );
    let mut settings = Settings::new("fixture-router-key".into(), "fixture/model".into()).unwrap();
    settings
        .select_backend("openai-api", "fixture-api-key".into())
        .unwrap();
    settings.model = "gpt-6.1-sol".into();
    let store = Store::new(directory.path().join("config"));
    store.save(&settings).unwrap();
    let agent = Agent::with_backend(
        services.create("gpt-6.1-sol".into()).unwrap(),
        Tools::new(directory.path()).unwrap(),
    );
    let mut app = App::new(agent, SessionConfig { store, settings }, None);
    app.dispatch("/model".into()).unwrap();
    app.choose(selector::Choice::ProviderGroup(
        selector::ProviderGroup::ApiKeys,
    ))
    .unwrap();
    app.choose(selector::Choice::Provider("openai-api"))
        .unwrap();
    wait(&mut app);
    let models = app.state.selector.as_ref().unwrap();
    let index = models
        .options
        .iter()
        .position(|row| row.name == "gpt-6.1-sol")
        .unwrap();
    app.choose(selector::Choice::Model(index)).unwrap();
    let efforts = app.state.selector.as_ref().unwrap();
    assert!(
        efforts
            .options
            .iter()
            .any(|row| matches!(row.choice, selector::Choice::Effort(Effort::High)))
    );
    assert!(
        efforts
            .options
            .iter()
            .any(|row| matches!(row.choice, selector::Choice::Effort(Effort::Max)))
    );
    app.choose(selector::Choice::Effort(Effort::High)).unwrap();
    assert_eq!(app.agent.as_ref().unwrap().backend_id(), "openai-api");
    assert_eq!(app.agent.as_ref().unwrap().model(), "gpt-6.1-sol");
    assert_eq!(app.agent.as_ref().unwrap().effort(), Effort::High);
    assert_eq!(app.config.settings.effort, Effort::Default);
    assert_eq!(fixture.finish().len(), 1);
}
