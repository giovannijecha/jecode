use super::*;
use crate::{
    config::{Settings, Store},
    json::Value,
    openrouter::{OpenRouter, Routing},
    test_support::{Directory, HttpFixture, completion},
    tools::Tools,
};
use std::{collections::BTreeMap, thread, time::Instant};

/// `notools/x` cannot serve tool requests, so it is listed last and blocked.
fn listing() -> Value {
    crate::json::parse(
        r#"{"data":{"endpoints":[
        {"tag":"one/fp8","provider_name":"One","pricing":{"prompt":"0.0000003","completion":"0.0000012"},
        "supported_parameters":["tools"]},
        {"tag":"notools/x","supported_parameters":[]},
        {"tag":"two/fp8","supported_parameters":["tools"]}
        ]}}"#,
    )
    .unwrap()
}

/// Isolated configuration and session storage, with routing set up as at startup.
fn app(home: &Directory, directory: &Directory, fixture: &HttpFixture, routing: Routing) -> App {
    let settings = Settings::new("isolated-fixture-key".into(), "fixture/model".into()).unwrap();
    let store = Store::new(home.path().join("config"));
    store.save(&settings).unwrap();
    let mut agent = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(directory.path()).unwrap(),
    );
    agent.set_routing_preferences(BTreeMap::new());
    agent.set_routing(routing);
    agent.enable_sessions(home.path()).unwrap();
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

fn shown(app: &App) -> String {
    view::frame(&app.state, "fixture/model", "directory")
        .live
        .iter()
        .map(|line| line.plain())
        .collect::<Vec<_>>()
        .join("\n")
}

fn notice(app: &App) -> &str {
    &app.state.notice.as_ref().unwrap().text
}

/// The latest local command result shown in the conversation.
fn result(app: &App) -> Option<&str> {
    app.state.items.iter().rev().find_map(|item| match item {
        state::Item::Local { result, .. } => result.as_deref(),
        _ => None,
    })
}

fn routing(app: &App) -> Routing {
    app.agent.as_ref().unwrap().routing()
}

#[test]
fn opening_routing_refreshes_an_expired_listing() {
    let home = Directory::new();
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![(200, listing())]);
    let mut app = app(&home, &directory, &fixture, Routing::Auto);
    app.agent
        .as_mut()
        .unwrap()
        .offer_endpoints(crate::backend::Endpoints {
            model: "fixture/model".into(),
            list: vec![],
            fetched: Instant::now() - crate::openrouter::routing::policy::FRESH,
        });
    app.dispatch("/routing".into()).unwrap();
    assert_eq!(
        app.state.selector.as_ref().unwrap().title,
        "Loading endpoints…"
    );
    wait(&mut app);
    assert!(
        app.state
            .selector
            .as_ref()
            .unwrap()
            .options
            .iter()
            .any(|option| option.name == "one/fp8")
    );
    assert_eq!(fixture.finish().len(), 1);
}

#[test]
fn routing_focuses_the_current_choice_and_applies_from_the_next_request() {
    let home = Directory::new();
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![(200, listing()), (200, completion("Answer", vec![]))]);
    let mut app = app(
        &home,
        &directory,
        &fixture,
        Routing::Endpoint("two/fp8".into()),
    );
    app.state.editor.insert("kept draft");
    app.dispatch("/routing".into()).unwrap();
    assert_eq!(
        app.state.selector.as_ref().unwrap().title,
        "Loading endpoints…"
    );
    wait(&mut app);
    let menu = app.state.selector.as_ref().unwrap();
    assert_eq!(menu.title, "Routing · fixture/model");
    let names: Vec<_> = menu
        .options
        .iter()
        .map(|option| option.name.as_str())
        .collect();
    assert_eq!(
        names,
        ["Auto — Best balance", "one/fp8", "two/fp8", "notools/x"]
    );
    assert_eq!(menu.selected, 2);
    assert!(menu.options[2].description.starts_with("current · "));
    assert!(menu.options[3].description.starts_with("no tools"));
    let displayed = shown(&app);
    for expected in [
        "Pinned to two/fp8 without fallbacks",
        "$/M input/output after discounts",
        "$0.3/$1.2",
    ] {
        assert!(
            displayed.contains(expected),
            "missing {expected}:\n{displayed}"
        );
    }
    assert!(!displayed.contains("kept draft"));
    // Moving focus keeps the current marker on the choice in effect.
    app.state.selector.as_mut().unwrap().selected = 0;
    let displayed = shown(&app);
    assert!(displayed.contains("Native routing with fallbacks · too little fresh performance"));
    assert!(
        app.state.selector.as_ref().unwrap().options[2]
            .description
            .starts_with("current · ")
    );
    app.input(Decoded::Text("1".into())).unwrap();
    assert!(app.state.selector.is_none());
    assert_eq!(routing(&app), Routing::Auto);
    assert_eq!(
        notice(&app),
        "Routing set to Auto — Best balance · was Endpoint two/fp8 · applies from the next request"
    );
    let session = app.agent.as_ref().unwrap().sessions().unwrap();
    assert_eq!(session.snapshot().routing, Routing::Auto);
    assert_eq!(app.config.settings.routing, BTreeMap::new());
    assert_eq!(app.state.editor.text, "kept draft");

    app.state.editor.take();
    app.input(Decoded::Text("question".into())).unwrap();
    enter(&mut app);
    wait(&mut app);
    let requests = fixture.finish();
    // The selector's listing serves the turn, so it is fetched once.
    assert_eq!(requests.len(), 2);
    assert!(requests[1].body.get("provider").is_none());
    assert_eq!(
        requests[1].body.get("session_id").and_then(Value::as_str),
        Some(session.id().as_str())
    );
}

#[test]
fn without_a_listing_auto_and_the_current_pin_remain_and_blocked_endpoints_keep_the_menu() {
    let home = Directory::new();
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![
        (
            500,
            Value::object([("error", Value::string("Fixture listing unavailable"))]),
        ),
        (200, listing()),
    ]);
    let pin = Routing::Endpoint("two/fp8".into());
    let mut app = app(&home, &directory, &fixture, pin.clone());
    app.dispatch("/routing".into()).unwrap();
    wait(&mut app);
    let menu = app.state.selector.as_ref().unwrap();
    let rows: Vec<_> = menu
        .options
        .iter()
        .map(|option| (option.name.as_str(), option.description.as_str()))
        .collect();
    assert_eq!(
        rows,
        [
            ("Auto — Best balance", "native routing with fallbacks"),
            ("two/fp8", "current · listing unavailable"),
        ]
    );
    assert!(shown(&app).contains("Endpoint listing unavailable · "));
    assert!(shown(&app).contains("HTTP 500"));
    app.close_selector();
    assert!(app.state.selector.is_none());
    assert_eq!(
        result(&app),
        Some("Selection cancelled · previous settings kept")
    );

    app.dispatch("/routing".into()).unwrap();
    wait(&mut app);
    app.input(Decoded::Text("4".into())).unwrap();
    let menu = app.state.selector.as_ref().unwrap();
    assert!(matches!(menu.purpose, selector::Purpose::Routing { .. }));
    assert_eq!(
        notice(&app),
        "notools/x does not accept tool calls, which every Jecode request includes · routing unchanged"
    );
    assert_eq!(routing(&app), pin.clone());
    assert_eq!(fixture.finish().len(), 2);
}

#[test]
fn routing_selection_keeps_the_current_choice_when_reasoning_is_unsupported() {
    let home = Directory::new();
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![(200, listing())]);
    let mut app = app(&home, &directory, &fixture, Routing::Auto);
    app.agent
        .as_mut()
        .unwrap()
        .set_effort(crate::effort::Effort::High);
    app.dispatch("/routing".into()).unwrap();
    wait(&mut app);
    app.input(Decoded::Text("2".into())).unwrap();
    assert!(app.state.selector.is_some());
    assert_eq!(routing(&app), Routing::Auto);
    assert!(notice(&app).contains("does not accept reasoning"));
    assert_eq!(fixture.finish().len(), 1);
}

#[test]
fn escape_cancels_the_listing_request_and_results_for_a_previous_model_are_discarded() {
    let home = Directory::new();
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![]);
    let mut app = app(&home, &directory, &fixture, Routing::Auto);
    app.state.editor.insert("kept draft");
    app.dispatch("/routing".into()).unwrap();
    assert!(app.job.is_some());
    app.close_selector();
    assert!(app.job.is_none());
    assert!(app.state.selector.is_none());
    assert_eq!(
        result(&app),
        Some("Selection cancelled · previous settings kept")
    );
    assert_eq!(app.state.editor.text, "kept draft");

    app.show_routing(
        jobs::RoutingPurpose {
            defaults: false,
            model: "fixture/previous".into(),
        },
        Err("unused".into()),
    );
    assert!(app.state.selector.is_none());
    assert_eq!(
        notice(&app),
        "Endpoints for fixture/previous discarded · the model changed meanwhile"
    );
    assert_eq!(routing(&app), Routing::Auto);
    assert!(fixture.finish().is_empty());
}

#[test]
fn default_routing_is_saved_per_model_without_changing_the_conversation_or_a_failed_save() {
    let home = Directory::new();
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![(200, listing())]);
    let mut app = app(&home, &directory, &fixture, Routing::Auto);
    app.dispatch("/settings".into()).unwrap();
    app.input(Decoded::Text("3".into())).unwrap();
    wait(&mut app);
    let menu = app.state.selector.as_ref().unwrap();
    assert_eq!(menu.title, "Default routing · fixture/model");
    assert_eq!(menu.selected, 0);
    app.input(Decoded::Text("2".into())).unwrap();
    let pin = Routing::Endpoint("one/fp8".into());
    let saved = app.config.store.load().unwrap().unwrap();
    assert_eq!(saved.routing.get("fixture/model"), Some(&pin));
    assert_eq!(app.config.settings.routing, saved.routing);
    assert_eq!(
        notice(&app),
        "Default routing for fixture/model: Endpoint one/fp8 · was Auto — Best balance · applies to new conversations and model switches"
    );
    assert_eq!(routing(&app), Routing::Auto);
    let menu = app.state.selector.as_ref().unwrap();
    assert!(matches!(menu.purpose, selector::Purpose::Settings));
    assert_eq!(menu.selected, 2);
    assert_eq!(menu.options[2].description, "Endpoint one/fp8");

    let path = app.config.store.path().to_path_buf();
    let original = std::fs::read(&path).unwrap();
    let permissions = std::fs::metadata(&path).unwrap().permissions();
    let mut readonly = permissions.clone();
    readonly.set_readonly(true);
    std::fs::set_permissions(&path, readonly).unwrap();
    // The listing is still fresh, so the menu opens without a request.
    app.input(Decoded::Text("3".into())).unwrap();
    assert!(app.job.is_none());
    assert_eq!(app.state.selector.as_ref().unwrap().selected, 1);
    app.input(Decoded::Text("1".into())).unwrap();
    std::fs::set_permissions(&path, permissions).unwrap();
    assert!(matches!(
        app.state.notice.as_ref().unwrap().kind,
        Kind::Error
    ));
    assert_eq!(std::fs::read(&path).unwrap(), original);
    assert_eq!(app.config.settings.routing.get("fixture/model"), Some(&pin));
    assert!(matches!(
        app.state.selector.as_ref().unwrap().purpose,
        selector::Purpose::Settings
    ));
    assert_eq!(routing(&app), Routing::Auto);
    assert_eq!(fixture.finish().len(), 1);
}

#[test]
fn routing_menus_fit_narrow_and_short_terminals_and_keep_the_draft() {
    let home = Directory::new();
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![(200, listing())]);
    let mut app = app(
        &home,
        &directory,
        &fixture,
        Routing::Endpoint("two/fp8".into()),
    );
    app.state.editor.insert("kept 🙂 draft");
    let draft = app.state.editor.clone();
    app.dispatch("/routing".into()).unwrap();
    wait(&mut app);
    for width in [2, 3, 8, 12, 18, 40, 80, 120] {
        for height in [1, 2, 4, 6, 8, 12, 24] {
            app.state.width = width;
            app.state.height = height;
            let frame = view::frame(&app.state, "fixture/model", "directory");
            assert!(frame.live.len() <= height, "{width}x{height}");
            assert!(
                frame
                    .live
                    .iter()
                    .all(|line| text::cells(&line.plain()) <= width),
                "{width}x{height}"
            );
            assert_eq!(app.state.editor, draft);
        }
    }
    // Short windows keep the focused option, then up to three options, ahead
    // of the help and then the notes.
    app.state.width = 80;
    for (height, options, help, legend) in [
        (4, 1, false, false),
        (6, 2, false, false),
        (8, 3, true, false),
        (9, 3, true, true),
    ] {
        app.state.height = height;
        let displayed = shown(&app);
        let rows = displayed
            .lines()
            .filter(|line| {
                line.trim_start_matches([' ', '›'])
                    .split_once(". ")
                    .is_some_and(|(number, _)| number.parse::<usize>().is_ok())
            })
            .count();
        assert_eq!(rows, options, "{displayed}");
        assert!(displayed.contains("› 3. two/fp8"), "{displayed}");
        assert_eq!(displayed.contains("Pinned to two/fp8"), help, "{displayed}");
        assert_eq!(
            displayed.contains("$/M input/output"),
            legend,
            "{displayed}"
        );
    }
    fixture.finish();
}
