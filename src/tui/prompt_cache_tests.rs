use super::*;
use crate::{
    backend::{CacheRetention, openrouter::OpenRouter},
    config::{Settings, Store},
    test_support::{Directory, HttpFixture},
    tools::Tools,
};

fn app(directory: &Directory) -> App {
    let settings = Settings::new("isolated-fixture-key".into(), "fixture/model".into()).unwrap();
    let store = Store::new(directory.path().join("config"));
    store.save(&settings).unwrap();
    let fixture = HttpFixture::new(vec![]);
    App::new(
        Agent::new(
            OpenRouter::fixture(fixture.endpoint.clone()),
            Tools::new(directory.path()).unwrap(),
        ),
        SessionConfig { store, settings },
        None,
    )
}

fn rows(app: &App) -> String {
    view::frame(&app.state, "fixture/model", "directory")
        .live
        .iter()
        .map(|line| line.plain())
        .collect::<Vec<_>>()
        .join("\n")
}

fn purpose(app: &App) -> &selector::Purpose {
    &app.state.selector.as_ref().unwrap().purpose
}

#[test]
fn one_hour_prompt_cache_saves_under_context_and_reaches_the_running_worker() {
    let directory = Directory::new();
    let mut app = app(&directory);
    let agent = app.hold();
    app.dispatch("/settings".into()).unwrap();
    assert!(rows(&app).contains("80% · 5 minutes"), "{}", rows(&app));
    app.input(Decoded::Text("5".into())).unwrap();
    assert!(matches!(purpose(&app), selector::Purpose::Context));
    let shown = rows(&app);
    assert!(shown.contains("Auto-compact at") && shown.contains("Prompt cache"));
    app.input(Decoded::Text("2".into())).unwrap();
    let menu = app.state.selector.as_ref().unwrap();
    assert!(matches!(menu.purpose, selector::Purpose::PromptCache));
    assert_eq!((menu.parent, menu.selected), (Some("Context"), 0));
    let shown = rows(&app);
    assert!(shown.contains("Claude models"), "{shown}");
    assert!(shown.contains("Survives breaks"), "{shown}");
    app.input(Decoded::Text("2".into())).unwrap();
    assert_eq!(agent.cache_retention(), CacheRetention::Long);
    assert_eq!(app.config.settings.cache_retention, CacheRetention::Long);
    assert_eq!(
        app.config.store.load().unwrap().unwrap().cache_retention,
        CacheRetention::Long
    );
    let notice = app.state.notice.as_ref().unwrap();
    assert!(notice.text.contains("Prompt cache: 5 minutes → 1 hour"));
    assert!(notice.change);
    assert!(matches!(purpose(&app), selector::Purpose::Context));
    assert_eq!(app.state.selector.as_ref().unwrap().selected, 1);
    assert!(rows(&app).contains("1 hour"));
    app.close_selector();
    assert!(matches!(purpose(&app), selector::Purpose::Settings));
    assert_eq!(app.state.selector.as_ref().unwrap().selected, 4);
    assert!(rows(&app).contains("80% · 1 hour"));
    app.agent = Some(agent);
}

#[test]
fn choosing_the_current_prompt_cache_returns_without_a_notice_or_a_write() {
    let directory = Directory::new();
    let mut app = app(&directory);
    let path = app.config.store.path().to_path_buf();
    let original = std::fs::read(&path).unwrap();
    app.dispatch("/settings".into()).unwrap();
    app.input(Decoded::Text("5".into())).unwrap();
    app.input(Decoded::Text("2".into())).unwrap();
    app.state.notice = None;
    app.input(Decoded::Text("1".into())).unwrap();
    assert!(app.state.notice.is_none());
    assert!(matches!(purpose(&app), selector::Purpose::Context));
    assert_eq!(std::fs::read(&path).unwrap(), original);
}
