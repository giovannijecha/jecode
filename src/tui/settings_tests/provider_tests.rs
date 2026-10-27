use super::*;

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
