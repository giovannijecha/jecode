use super::*;
use crate::{
    backend::openrouter::OpenRouter,
    config::{Settings, Store},
    test_support::{Directory, HttpFixture},
    tools::Tools,
};

fn app(directory: &Directory, fixture: &HttpFixture) -> App {
    let settings = Settings::new("isolated-fixture-key".into(), "fixture/model".into()).unwrap();
    let store = Store::new(directory.path().join("config"));
    store.save(&settings).unwrap();
    let mut client = OpenRouter::fixture(fixture.endpoint.clone());
    client.fixture_limits(24000, Some(4096));
    App::new(
        Agent::new(client, Tools::new(directory.path()).unwrap()),
        SessionConfig { store, settings },
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

fn open(app: &mut App) {
    app.dispatch("/settings".into()).unwrap();
    app.input(Decoded::Text("5".into())).unwrap();
    let menu = app.state.selector.as_ref().unwrap();
    assert!(matches!(menu.purpose, selector::Purpose::AutoCompact));
}

/// The active engine, the saved file and the footer agree on one value.
fn assert_preference(app: &App, percent: u8) {
    assert_eq!(app.config.settings.auto_compact, percent);
    assert_eq!(
        app.config.store.load().unwrap().unwrap().auto_compact,
        percent
    );
    let agent = app.agent.as_ref().unwrap().context_status();
    assert_eq!(agent.auto_compact, percent);
    assert_eq!(app.state.context, Some(agent));
}

#[test]
fn a_saved_threshold_applies_to_this_and_new_conversations() {
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![]);
    let mut app = app(&directory, &fixture);
    app.state.editor.insert("kept draft");
    assert_eq!(app.state.context.unwrap().capacity, Some(24000));
    open(&mut app);
    assert_eq!(app.state.selector.as_ref().unwrap().editor.text, "80");
    app.input(Decoded::Text("5".into())).unwrap();
    enter(&mut app);
    // "805" is rejected: the panel stays open and nothing changes.
    let menu = app.state.selector.as_ref().unwrap();
    assert!(matches!(menu.purpose, selector::Purpose::AutoCompact));
    let notice = app.state.notice.as_ref().unwrap();
    assert!(notice.text.contains("1 to 99 · previous setting kept"));
    assert_preference(&app, 80);
    app.state
        .selector
        .as_mut()
        .unwrap()
        .editor
        .set("55%".into());
    enter(&mut app);
    assert_preference(&app, 55);
    let menu = app.state.selector.as_ref().unwrap();
    assert!(matches!(menu.purpose, selector::Purpose::Settings));
    assert_eq!(menu.selected, 4);
    let rows = view::frame(&app.state, "fixture/model", "directory")
        .live
        .iter()
        .map(|line| line.plain())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(rows.contains("Auto-compact at") && rows.contains("55%"));
    // The footer meter now fills toward the new threshold.
    assert_eq!(
        app.state.context.as_ref().map(|status| status.auto_compact),
        Some(55)
    );
    let notice = app.state.notice.as_ref().unwrap();
    assert!(notice.text.contains("Auto-compact: 80% → 55%"));
    assert!(notice.change);
    app.close_selector();
    assert_eq!(app.state.editor.text, "kept draft");
    app.state.editor.take();
    app.dispatch("/new".into()).unwrap();
    assert_preference(&app, 55);
    assert!(fixture.finish().is_empty());
}

#[test]
fn cancelling_or_a_failed_save_keeps_the_previous_threshold_active_and_saved() {
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![]);
    let mut app = app(&directory, &fixture);
    open(&mut app);
    app.state.selector.as_mut().unwrap().editor.set("40".into());
    app.close_selector();
    let menu = app.state.selector.as_ref().unwrap();
    assert!(matches!(menu.purpose, selector::Purpose::Settings));
    assert_eq!(menu.selected, 4);
    assert_preference(&app, 80);
    app.close_selector();
    let path = app.config.store.path().to_path_buf();
    let original = std::fs::read(&path).unwrap();
    let permissions = std::fs::metadata(&path).unwrap().permissions();
    let mut readonly = permissions.clone();
    readonly.set_readonly(true);
    std::fs::set_permissions(&path, readonly).unwrap();
    open(&mut app);
    app.state.selector.as_mut().unwrap().editor.set("40".into());
    enter(&mut app);
    std::fs::set_permissions(&path, permissions).unwrap();
    assert!(matches!(
        app.state.notice.as_ref().unwrap().kind,
        Kind::Error
    ));
    assert_eq!(std::fs::read(&path).unwrap(), original);
    assert_preference(&app, 80);
    assert!(fixture.finish().is_empty());
}

#[test]
fn a_saved_threshold_reaches_the_worker_before_it_returns_the_agent() {
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![]);
    let mut app = app(&directory, &fixture);
    let agent = app.hold();
    assert!(app.agent.is_none());
    open(&mut app);
    app.state.selector.as_mut().unwrap().editor.set("55".into());
    enter(&mut app);
    assert_eq!(agent.context_status().auto_compact, 55);
    assert_eq!(app.state.context.unwrap().auto_compact, 55);
    let mut queued = app.state.context.unwrap();
    queued.auto_compact = 80;
    app.worker_event(crate::events::Event::Context(queued));
    assert_eq!(app.state.context.unwrap().auto_compact, 55);
    assert_eq!(app.config.store.load().unwrap().unwrap().auto_compact, 55);
    assert!(app.deferred.is_empty());
    app.agent = Some(agent);
    app.apply_deferred();
    app.refresh_context();
    assert_preference(&app, 55);
    assert!(fixture.finish().is_empty());
}
