use super::super::{
    App, Decoded, SessionConfig, selector, terminal,
    tests::{config, finish, wait_until},
    view,
};
use crate::{
    agent::Agent,
    backend::openrouter::OpenRouter,
    json::Value,
    permissions::{Access, Kind, Level},
    test_support::{Directory, HttpFixture, completion, tool_call},
    tools::Tools,
};
use std::fs;

fn app(directory: &Directory, fixture: &HttpFixture) -> App {
    let agent = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(directory.path()).unwrap(),
    );
    let SessionConfig { store, settings } = config(directory);
    store.save(&settings).unwrap();
    App::new(agent, SessionConfig { store, settings }, None)
}

fn key(app: &mut App, code: u16, modifiers: u8) {
    app.input(Decoded::Key(terminal::Key {
        code,
        modifiers,
        character: 0,
    }))
    .unwrap();
}

fn text(app: &mut App, value: &str) {
    app.input(Decoded::Text(value.into())).unwrap();
}

fn screen(app: &App) -> String {
    view::frame(&app.state, "fixture/model", "directory")
        .live
        .iter()
        .map(|line| line.plain())
        .collect::<Vec<_>>()
        .join("\n")
}

fn notice(app: &App) -> String {
    app.state.notice.as_ref().unwrap().text.clone()
}

fn outside(kind: Kind, target: &str) -> Access {
    Access {
        kind,
        target: target.into(),
        place: None,
        outside: true,
        base: None,
        identity: None,
    }
}

#[test]
fn the_level_menu_applies_at_once_and_the_composer_names_the_level() {
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![]);
    let mut app = app(&directory, &fixture);
    assert!(screen(&app).contains("Workspace ─"));
    app.dispatch("/permissions".into()).unwrap();
    let menu = app.state.selector.as_ref().unwrap();
    assert!(matches!(
        menu.purpose,
        selector::Purpose::Permissions {
            default: Level::Workspace
        }
    ));
    assert_eq!(menu.selected, 1, "the focus opens on the current level");
    let shown = screen(&app);
    assert!(shown.contains("Edit and run here and in subfolders; network on"));
    assert!(shown.contains("Remembered") && shown.contains("none"));
    assert!(shown.contains("new conversations use Workspace"));
    // The current level closes with no notice and no change.
    text(&mut app, "2");
    assert!(app.state.selector.is_none() && app.state.notice.is_none());
    app.dispatch("/permissions".into()).unwrap();
    text(&mut app, "1");
    assert_eq!(app.permission_observer.level(), Level::Read);
    assert!(notice(&app).contains("Permissions: Workspace → Read · this conversation"));
    assert!(screen(&app).contains("Read ─"));
    // Remembered opens empty and Esc returns to its row.
    app.dispatch("/permissions".into()).unwrap();
    text(&mut app, "4");
    assert!(screen(&app).contains("Nothing remembered"));
    key(&mut app, 27, 0);
    let menu = app.state.selector.as_ref().unwrap();
    assert!(matches!(
        menu.purpose,
        selector::Purpose::Permissions { .. }
    ));
    assert_eq!(menu.selected, 3);
    key(&mut app, 27, 0);
    assert!(app.state.selector.is_none());
    app.dispatch("/permissions full".into()).unwrap();
    assert_eq!(app.permission_observer.level(), Level::Full);
}

#[test]
fn the_saved_default_leaves_this_conversation_level_alone() {
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![]);
    let mut app = app(&directory, &fixture);
    app.permission_observer.set_level(Level::Full).unwrap();
    app.dispatch("/settings".into()).unwrap();
    assert!(screen(&app).contains("Default permissions"));
    text(&mut app, "4");
    let menu = app.state.selector.as_ref().unwrap();
    assert!(matches!(
        menu.purpose,
        selector::Purpose::DefaultPermissions
    ));
    text(&mut app, "1");
    assert_eq!(
        app.config
            .store
            .load()
            .unwrap()
            .unwrap()
            .default_permissions,
        Level::Read
    );
    assert_eq!(app.permission_observer.default_level(), Level::Read);
    assert_eq!(app.permission_observer.level(), Level::Full);
    assert!(
        notice(&app)
            .contains("Default permissions: Workspace → Read · this conversation stays Full")
    );
    let menu = app.state.selector.as_ref().unwrap();
    assert!(matches!(menu.purpose, selector::Purpose::Settings));
    assert_eq!(
        menu.selected,
        menu.setting_index(selector::Setting::Permissions)
    );
}

#[test]
fn a_request_holds_the_keys_and_a_remembered_grant_can_be_revoked() {
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![]);
    let mut app = app(&directory, &fixture);
    app.state.editor.insert("kept draft");
    let collaborator = app.permission_observer.for_agent("a1");
    let read = outside(Kind::Reads, r"C:\elsewhere\session.rs");
    let first = collaborator.begin("child-call", "read", &read).unwrap();
    app.permission_observer
        .begin("main-call", "bash", &outside(Kind::Runs, "cargo test"))
        .unwrap();
    app.poll().unwrap();
    let shown = screen(&app);
    for expected in [
        "Waiting for permission",
        "Permission",
        "AGENT 1",
        "· 1 of 2",
        "READ",
        "Read access outside this folder",
        "Only this read",
        "Reads of this file, this conversation",
        "Nothing runs; the agent is told",
        "1–3  choose",
    ] {
        assert!(shown.contains(expected), "{expected} in\n{shown}");
    }
    assert!(!shown.contains("kept draft"), "the draft waits, hidden");
    // Letters and Enter alone decide nothing; ↓ then Enter does.
    text(&mut app, "y");
    key(&mut app, 13, 0);
    assert_eq!(app.permission_observer.pending().unwrap().0, first);
    assert_eq!(app.state.editor.text, "kept draft");
    key(&mut app, 40, 0);
    key(&mut app, 40, 0);
    assert!(screen(&app).contains("Enter  choose"));
    key(&mut app, 13, 0);
    assert_eq!(app.permission_observer.grants(), vec![read.grant()]);
    assert!(notice(&app).contains("Permission: Reads of this file · remembered"));
    // The main agent's request follows, with the asking agent's badge.
    let shown = screen(&app);
    assert!(shown.contains("MAIN AGENT") && !shown.contains("1 of"));
    text(&mut app, "3");
    assert!(app.state.permission.is_none());
    assert!(screen(&app).contains("kept draft"));

    app.dispatch("/permissions".into()).unwrap();
    assert!(screen(&app).contains("1 for this conversation"));
    text(&mut app, "4");
    assert!(screen(&app).contains("Reads") && screen(&app).contains("session.rs"));
    key(&mut app, 68, 4);
    assert!(app.permission_observer.grants().is_empty());
    assert!(notice(&app).contains("Revoked: session.rs · reads ask again"));
    assert!(screen(&app).contains("Nothing remembered"));
}

#[test]
fn an_approved_outside_read_runs_inside_a_turn() {
    let directory = Directory::new();
    let other = Directory::new();
    let path = other.path().join("session.rs");
    fs::write(&path, "wall-clock times\n").unwrap();
    let read = tool_call(
        "read-1",
        "read",
        Value::object([("path", Value::string(path.to_string_lossy()))]),
    );
    let fixture = HttpFixture::new(vec![
        (200, completion("", vec![read])),
        (200, completion("Compared", vec![])),
    ]);
    let mut app = app(&directory, &fixture);
    app.state.editor.insert("Compare webterminal");
    app.edited();
    assert!(!app.submit().unwrap());
    wait_until(&mut app, |app| app.state.permission.is_some());
    let pending = app.state.permission.clone().unwrap();
    assert_eq!(pending.agent, "MAIN AGENT");
    text(&mut app, "1");
    finish(&mut app);
    let requests = fixture.finish();
    let result = super::super::tests::last_content(&requests[1]);
    assert!(result.contains("wall-clock"), "{result}");
}

#[test]
fn the_level_changes_while_a_turn_holds_the_agent() {
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![]);
    let mut app = app(&directory, &fixture);
    let agent = app.hold();
    app.dispatch("/permissions".into()).unwrap();
    text(&mut app, "1");
    assert!(notice(&app).contains("Permissions: Workspace → Read · from the next call"));
    // The running agent reads the new level before its next call.
    assert_eq!(agent.permissions().level(), Level::Read);
    app.agent = Some(agent);
    app.held = None;
}

#[test]
fn permission_display_masks_secrets_and_paste_never_approves() {
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![]);
    let mut app = app(&directory, &fixture);
    let secret = "permission-fixture-secret";
    app.archive.redactor.add_secret(secret.into());
    let access = outside(Kind::Runs, &format!("program --token {secret}"));
    let request = app
        .permission_observer
        .begin("sensitive", "exec", &access)
        .unwrap();
    app.poll().unwrap();
    assert!(!screen(&app).contains(secret), "{}", screen(&app));
    app.input(Decoded::Paste("2".into())).unwrap();
    assert_eq!(app.permission_observer.pending().unwrap().0.id, request.id);
    key(&mut app, 40, 0);
    app.poll().unwrap();
    assert_eq!(app.state.permission.as_ref().unwrap().focus, Some(0));
    text(&mut app, "2");
    assert_eq!(app.permission_observer.grants(), vec![access.grant()]);
    app.dispatch("/permissions".into()).unwrap();
    text(&mut app, "4");
    assert!(!screen(&app).contains(secret), "{}", screen(&app));
    key(&mut app, 68, 4);
    assert!(app.permission_observer.grants().is_empty());
}
