use super::*;
use crate::test_support::Directory;

fn saved(directory: &Directory) -> (Store, Settings) {
    let store = Store::new(directory.path().to_path_buf());
    let mut settings =
        Settings::new("isolated-fixture-key".into(), "fixture/saved".into()).unwrap();
    settings.effort = Effort::High;
    store.save(&settings).unwrap();
    (store, settings)
}

#[test]
fn new_conversations_take_run_options_over_saved_defaults() {
    let directory = Directory::new();
    let (store, settings) = saved(&directory);
    let none = for_new(&settings, &store, &Selection::default()).unwrap();
    assert_eq!(
        (none.provider.as_str(), none.model.as_str(), none.effort),
        ("openrouter", "fixture/saved", Effort::High)
    );
    let model = for_new(
        &settings,
        &store,
        &Selection {
            model: Some("fixture/other".into()),
            ..Selection::default()
        },
    )
    .unwrap();
    assert_eq!(
        (model.model.as_str(), model.effort),
        ("fixture/other", Effort::Default)
    );
    let effort = for_new(
        &settings,
        &store,
        &Selection {
            effort: Some(Effort::Low),
            ..Selection::default()
        },
    )
    .unwrap();
    assert_eq!(
        (effort.model.as_str(), effort.effort),
        ("fixture/saved", Effort::Low)
    );
    // Another provider needs its own saved model or an explicit one.
    let error = for_new(
        &settings,
        &store,
        &Selection {
            provider: Some("openai-api".into()),
            ..Selection::default()
        },
    )
    .err()
    .unwrap();
    assert!(error.contains("--model"), "{error}");
}

#[test]
fn explicit_providers_report_their_own_missing_credentials() {
    let directory = Directory::new();
    let (_, settings) = saved(&directory);
    assert!(services(&settings, directory.path(), "openrouter").is_ok());
    let error = services(&settings, directory.path(), "openai-api")
        .err()
        .unwrap();
    assert!(error.contains("OpenAI API") && error.contains("providers login openai-api"));
    let error = services(&settings, directory.path(), "openai")
        .err()
        .unwrap();
    assert!(error.contains("ChatGPT") && error.contains("providers login openai"));
}

#[test]
fn routing_pins_require_a_listed_compatible_endpoint() {
    let listing = crate::json::parse(
        r#"{"data":{"endpoints":[{"tag":"fixture/ready","supported_parameters":["tools","reasoning"]},{"tag":"fixture/no-tools","supported_parameters":[]}]}}"#,
    )
    .unwrap();
    let fixture = crate::test_support::HttpFixture::new(vec![
        (200, listing.clone()),
        (200, listing.clone()),
        (200, listing),
    ]);
    let services = Services::fixture(&fixture.endpoint);
    let ready = Routing::Endpoint("fixture/ready".into());
    let endpoints = check_routing(&services, "fixture/model", Effort::Low, &ready)
        .unwrap()
        .unwrap();
    assert!(endpoints.find("fixture/ready").is_some());
    let missing = check_routing(
        &services,
        "fixture/model",
        Effort::Default,
        &Routing::Endpoint("fixture/missing".into()),
    )
    .err()
    .unwrap();
    assert!(missing.contains("has no endpoint"), "{missing}");
    let unsupported = check_routing(
        &services,
        "fixture/model",
        Effort::Default,
        &Routing::Endpoint("fixture/no-tools".into()),
    )
    .err()
    .unwrap();
    assert!(
        unsupported.contains("does not accept tool calls"),
        "{unsupported}"
    );
    assert_eq!(fixture.finish().len(), 3);
}

#[test]
fn explicit_auto_route_is_installed_before_a_stale_saved_pin_is_checked() {
    let directory = Directory::new();
    let fixture = crate::test_support::HttpFixture::new(vec![(
        200,
        crate::json::parse(r#"{"data":{"endpoints":[]}}"#).unwrap(),
    )]);
    let mut agent = fixture_agent(&directory, &fixture.endpoint);
    agent.set_backend_routing_preferences(
        "openrouter",
        std::collections::BTreeMap::from([(
            "fixture/other".into(),
            Routing::Endpoint("fixture/stale".into()),
        )]),
    );
    let next = Services::fixture(&fixture.endpoint)
        .create("fixture/other".into())
        .unwrap();
    agent
        .replace_backend_with_routing(next, Routing::Auto)
        .unwrap();
    assert_eq!(
        (agent.model(), agent.routing()),
        ("fixture/other", Routing::Auto)
    );
    assert_eq!(fixture.finish().len(), 1);
}

#[test]
fn explicit_provider_restores_history_when_saved_credentials_are_missing() {
    let directory = Directory::new();
    let fixture = crate::test_support::HttpFixture::new(vec![]);
    let mut agent = fixture_agent(&directory, &fixture.endpoint);
    let mut document = crate::sessions::Document::new(
        directory.path().to_path_buf(),
        "fixture/old".into(),
        Effort::Low,
    )
    .unwrap();
    document.backend = "openai-api".into();
    document.permissions = Some(crate::permissions::Level::Read);
    document.messages = agent.sessions().unwrap().snapshot().messages;
    document.messages.push(crate::json::Value::object([
        ("role", crate::json::Value::string("user")),
        ("content", crate::json::Value::string("Keep this history")),
    ]));
    let id = document.id.clone();
    let saved = crate::sessions::Handle::new(
        agent.sessions().unwrap().store(),
        document,
        agent.redactor(),
    );
    saved.flush().unwrap();
    drop(saved);
    let error = agent.resume(&id).unwrap_err();
    assert!(
        error.contains("openai-api has no configured account"),
        "{error}"
    );
    let home = directory.path().join(".jecode");
    let store = Store::new(home);
    let settings = Settings::new("isolated-fixture-key".into(), "fixture/new".into()).unwrap();
    store.save(&settings).unwrap();
    let original = std::fs::read(store.path()).unwrap();
    let before = agent.sessions().unwrap().id();
    let incompatible = resume(
        &mut agent,
        &id,
        &Selection {
            provider: Some("openai-api".into()),
            model: Some("fixture/new".into()),
            routing: Some(Routing::Auto),
            ..Selection::default()
        },
        &settings,
        &store,
    )
    .unwrap_err();
    assert!(incompatible.contains("only available with openrouter"));
    assert_eq!(agent.sessions().unwrap().id(), before);
    assert_eq!(
        agent.sessions().unwrap().store().read(&id).unwrap().backend,
        "openai-api"
    );
    let status = resume(
        &mut agent,
        &id,
        &Selection {
            provider: Some("openrouter".into()),
            model: Some("fixture/new".into()),
            routing: Some(Routing::Auto),
            ..Selection::default()
        },
        &settings,
        &store,
    )
    .unwrap();
    assert!(status.contains("fixture/new"));
    assert_eq!(agent.backend_id(), "openrouter");
    assert_eq!(agent.model(), "fixture/new");
    assert_eq!(agent.routing(), Routing::Auto);
    assert_eq!(agent.permissions().level(), crate::permissions::Level::Read);
    let restored = agent.sessions().unwrap().snapshot();
    assert_eq!(restored.compatible_from, restored.messages.len());
    assert!(restored.messages.iter().any(|message| {
        message.get("content").and_then(crate::json::Value::as_str) == Some("Keep this history")
    }));
    assert_eq!(std::fs::read(store.path()).unwrap(), original);
    assert_eq!(fixture.finish().len(), 0);
}

#[test]
fn replacing_missing_credentials_records_the_saved_routing() {
    let directory = Directory::new();
    let fixture = crate::test_support::HttpFixture::new(vec![]);
    let services = Services::from_service(crate::backend::openai::Api::fixture_api_key(
        "isolated-fixture-key",
        &fixture.endpoint,
    ));
    let mut agent = Agent::with_backend(
        services.create("fixture/current".into()).unwrap(),
        crate::tools::Tools::new(directory.path()).unwrap(),
    );
    let home = directory.path().join(".jecode");
    agent.enable_sessions(&home).unwrap();
    let mut document = crate::sessions::Document::new(
        directory.path().to_path_buf(),
        "fixture/old".into(),
        Effort::Low,
    )
    .unwrap();
    document.routing = Routing::Endpoint("fixture/old-route".into());
    document.messages = agent.sessions().unwrap().snapshot().messages;
    document.messages.push(crate::json::Value::object([
        ("role", crate::json::Value::string("user")),
        ("content", crate::json::Value::string("Keep this history")),
    ]));
    let id = document.id.clone();
    let handle = crate::sessions::Handle::new(
        agent.sessions().unwrap().store(),
        document,
        agent.redactor(),
    );
    handle.flush().unwrap();
    drop(handle);
    let store = Store::new(home);
    let settings = Settings::new("isolated-fixture-key".into(), "fixture/new".into()).unwrap();
    store.save(&settings).unwrap();
    let status = resume(
        &mut agent,
        &id,
        &Selection {
            provider: Some("openrouter".into()),
            model: Some("fixture/new".into()),
            routing: Some(Routing::Auto),
            ..Selection::default()
        },
        &settings,
        &store,
    )
    .unwrap();
    assert!(
        status.contains("was Endpoint fixture/old-route"),
        "{status}"
    );
    assert_eq!(agent.routing(), Routing::Auto);
    assert_eq!(fixture.finish().len(), 0);
}

#[test]
fn the_most_recent_session_is_found_without_opening_it() {
    let directory = Directory::new();
    let project = directory.path().join("project");
    std::fs::create_dir(&project).unwrap();
    assert!(
        last_session(directory.path(), &project)
            .unwrap_err()
            .contains("No saved conversations")
    );
}

fn fixture_agent(directory: &Directory, endpoint: &str) -> Agent {
    let mut agent = Agent::new(
        crate::backend::openrouter::OpenRouter::fixture(endpoint.into()),
        crate::tools::Tools::new(directory.path()).unwrap(),
    );
    agent
        .enable_sessions(&directory.path().join(".jecode"))
        .unwrap();
    agent
}

#[test]
fn resumed_conversations_keep_their_selection_unless_a_run_option_names_it() {
    let directory = Directory::new();
    let fixture = crate::test_support::HttpFixture::new(vec![(
        200,
        crate::test_support::completion("Saved answer.", vec![]),
    )]);
    let home = directory.path().join(".jecode");
    let store = Store::new(home.clone());
    let settings = Settings::new("isolated-fixture-key".into(), "fixture/default".into()).unwrap();
    store.save(&settings).unwrap();
    let saved_config = std::fs::read(store.path()).unwrap();

    let mut first = fixture_agent(&directory, &fixture.endpoint);
    first.set_model("fixture/saved".into()).unwrap();
    first.set_effort(Effort::Low);
    first
        .permissions()
        .set_level(crate::permissions::Level::Read)
        .unwrap();
    first
        .run_turn(
            crate::attachments::Prompt::plain("remember this"),
            &mut |_event: crate::events::Event| Ok(()),
        )
        .unwrap();
    first.set_routing(Routing::Endpoint("fixture/pinned".into()));
    let handle = first.sessions().unwrap();
    first.save_session().unwrap();
    let id = handle.id();
    drop(handle);
    drop(first);
    assert_eq!(last_session(&home, directory.path()).unwrap(), id);

    // No run option: the saved conversation's own choices return.
    let mut resumed = fixture_agent(&directory, &fixture.endpoint);
    resumed.resume(&id).unwrap();
    assert_eq!(
        apply(&mut resumed, &Selection::default(), &settings, &store, None).unwrap(),
        None
    );
    assert_eq!(
        (
            resumed.model(),
            resumed.effort(),
            resumed.permissions().level(),
            resumed.routing()
        ),
        (
            "fixture/saved",
            Effort::Low,
            crate::permissions::Level::Read,
            Routing::Endpoint("fixture/pinned".into())
        )
    );
    drop(resumed);

    let mut resumed = fixture_agent(&directory, &fixture.endpoint);
    resumed.resume(&id).unwrap();
    let routed = apply(
        &mut resumed,
        &Selection {
            routing: Some(Routing::Auto),
            ..Selection::default()
        },
        &settings,
        &store,
        None,
    )
    .unwrap()
    .unwrap();
    assert!(
        routed.starts_with("Routing set to ") && routed.contains(" · was "),
        "{routed}"
    );
    assert_eq!(resumed.routing(), Routing::Auto);
    assert_eq!(std::fs::read(store.path()).unwrap(), saved_config);
    drop(resumed);

    // Named options change this conversation only.
    let mut resumed = fixture_agent(&directory, &fixture.endpoint);
    resumed.resume(&id).unwrap();
    let summary = apply(
        &mut resumed,
        &Selection {
            model: Some("fixture/override".into()),
            permissions: Some(crate::permissions::Level::Full),
            ..Selection::default()
        },
        &settings,
        &store,
        None,
    )
    .unwrap()
    .unwrap();
    assert!(summary.contains("fixture/override") && summary.contains("Full"));
    assert_eq!(
        (
            resumed.model(),
            resumed.effort(),
            resumed.permissions().level()
        ),
        (
            "fixture/override",
            Effort::Default,
            crate::permissions::Level::Full
        )
    );
    resumed.save_session().unwrap();
    let document = resumed.sessions().unwrap().snapshot();
    assert_eq!(document.model.as_deref(), Some("fixture/override"));
    assert_eq!(std::fs::read(store.path()).unwrap(), saved_config);
    assert_eq!(fixture.finish().len(), 1);
}
