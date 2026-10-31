use super::*;
use std::fs;

#[test]
fn account_selection_ignores_an_unrelated_corrupt_active_account() {
    for provider in ["openai", "anthropic"] {
        let directory = Directory::new();
        let fixture = HttpFixture::new(vec![]);
        let mut app = app(&directory, &fixture);
        let home = app.config.store.path().parent().unwrap().to_path_buf();
        let other = if provider == "openai" {
            crate::auth::openai::Store::fixture_account(home.clone(), "fixture-openai-token")
                .unwrap();
            "anthropic"
        } else {
            crate::auth::anthropic::Store::fixture_account(home.clone(), "fixture-claude-token")
                .unwrap();
            "openai"
        };
        let other_directory = home.join("accounts").join(other);
        fs::create_dir_all(&other_directory).unwrap();
        fs::write(other_directory.join("active.json"), "{broken").unwrap();

        app.open_accounts_for(provider);
        app.open_account(0);
        app.activate_account();

        assert!(
            app.agent.as_ref().unwrap().has_backend(provider),
            "{provider}"
        );
        assert!(
            app.state
                .notice
                .as_ref()
                .unwrap()
                .text
                .contains("account selected"),
            "{provider}"
        );
        assert!(fixture.finish().is_empty());
    }
}

#[test]
fn completed_sign_in_updates_its_origin_provider_even_after_menu_changes() {
    for provider in ["openai", "anthropic"] {
        let directory = Directory::new();
        let fixture = HttpFixture::new(vec![]);
        let mut app = app(&directory, &fixture);
        let home = app.config.store.path().parent().unwrap().to_path_buf();
        let (account, other) = if provider == "openai" {
            (
                crate::auth::openai::Store::fixture_account(home.clone(), "fixture-openai-token")
                    .unwrap()
                    .into(),
                "anthropic",
            )
        } else {
            (
                crate::auth::anthropic::Store::fixture_account(
                    home.clone(),
                    "fixture-claude-token",
                )
                .unwrap()
                .into(),
                "openai",
            )
        };
        let other_directory = home.join("accounts").join(other);
        fs::create_dir_all(&other_directory).unwrap();
        fs::write(other_directory.join("active.json"), "{broken").unwrap();

        app.auth_provider = other;
        app.finish_sign_in(provider, Ok(account));

        assert!(
            app.agent.as_ref().unwrap().has_backend(provider),
            "{provider}"
        );
        assert!(!app.agent.as_ref().unwrap().has_backend(other), "{other}");
        assert!(
            app.state
                .notice
                .as_ref()
                .unwrap()
                .text
                .contains("Connected"),
            "{provider}"
        );
        assert!(fixture.finish().is_empty());
    }
}

#[test]
fn deferred_account_refresh_keeps_the_provider_chosen_during_a_turn() {
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![]);
    let mut app = app(&directory, &fixture);
    let home = app.config.store.path().parent().unwrap().to_path_buf();
    crate::auth::openai::Store::fixture_account(home, "fixture-openai-token").unwrap();
    app.open_accounts_for("openai");
    app.open_account(0);

    let agent = app.hold();
    app.activate_account();
    app.auth_provider = "anthropic";
    app.agent = Some(agent);
    app.apply_deferred();

    assert!(app.agent.as_ref().unwrap().has_backend("openai"));
    assert!(!app.agent.as_ref().unwrap().has_backend("anthropic"));
    assert!(fixture.finish().is_empty());
}

#[test]
fn claude_account_selection_keeps_openrouter_conversation_and_default() {
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![]);
    let mut app = app(&directory, &fixture);
    let home = app.config.store.path().parent().unwrap().to_path_buf();
    let account =
        crate::auth::anthropic::Store::fixture_account(home.clone(), "isolated-claude-token")
            .unwrap();
    app.open_accounts_for("anthropic");
    let menu = app.state.selector.as_ref().unwrap();
    assert!(matches!(menu.purpose, selector::Purpose::Accounts));
    assert!(menu.options[0].name.contains("Fixture"));
    app.open_account(0);
    app.activate_account();
    assert_eq!(app.agent.as_ref().unwrap().backend_id(), "openrouter");
    assert_eq!(app.config.settings.backend_id, "openrouter");
    assert_eq!(app.archive.model, "fixture/model");
    assert_eq!(
        crate::auth::anthropic::Store::new(home)
            .active()
            .unwrap()
            .unwrap()
            .id(),
        account.id
    );
    assert!(app.job.is_none());
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
    let mut app = App::new(agent, SessionConfig { store, settings }, None);
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
    assert!(menu.options[index].fact.contains("fixture@example.test"));
    app.choose(selector::Choice::Provider("openai")).unwrap();
    assert!(index < 2);
    assert!(app.job.is_some());
    wait(&mut app);
    assert!(matches!(
        app.state.selector.as_ref().unwrap().purpose,
        selector::Purpose::Models { defaults: false }
    ));
    // The catalog's account rides the rule title.
    assert!(
        app.state
            .selector
            .as_ref()
            .unwrap()
            .context
            .contains("fixture@example.test")
    );
    // The long context needs a wide terminal; narrower ones drop it first.
    let rule = |app: &App| {
        view::frame(&app.state, "fixture/original", "directory")
            .live
            .iter()
            .map(|row| row.plain())
            .find(|row| row.starts_with("─ Provider › Model"))
            .unwrap()
    };
    assert!(!rule(&app).contains("fixture@example.test"));
    app.state.width = 120;
    assert!(rule(&app).contains("· fixture@example.test"));
    app.state.width = 80;
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
