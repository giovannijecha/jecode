//! A question or permission request that arrives while the draft is typed in
//! leaves the rest of that typing to the draft.
use super::*;
use crate::{
    ask::Request,
    permissions::{Access, Kind as AccessKind},
    tui::{keys::Decoded, terminal::Key},
};

fn key(app: &mut App, code: u16) {
    app.input(Decoded::Key(Key {
        code,
        modifiers: 0,
        character: 0,
    }))
    .unwrap();
}

fn text(app: &mut App, value: &str) {
    app.input(Decoded::Text(value.into())).unwrap();
}

fn app(directory: &Directory, home: &Directory, fixture: &HttpFixture) -> App {
    let mut agent = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(directory.path()).unwrap(),
    );
    agent.enable_sessions(home.path()).unwrap();
    App::new(agent, config(directory), None)
}

fn single() -> Request {
    let arguments = json::parse(
        r#"{"questions":[{"id":"scope","title":"Scope","question":"Which rows?",
        "options":[{"id":"open","label":"Open rows"},{"id":"all","label":"All rows"}]}]}"#,
    )
    .unwrap();
    Request::parse("waiting", &arguments).unwrap()
}

#[test]
fn a_question_arriving_mid_typing_answers_nothing_until_a_deliberate_key() {
    let (directory, home) = (Directory::new(), Directory::new());
    let fixture = HttpFixture::new(vec![]);
    let mut app = app(&directory, &home, &fixture);
    text(&mut app, "fix bu");
    assert!(app.question_observer.begin(single()).unwrap());
    app.refresh_question();
    let panel = app.state.ask_panel.as_ref().unwrap();
    assert!(panel.guard.is_some());
    assert_eq!(panel.entry().row, usize::MAX, "nothing is focused");
    // The rest of the typing, a digit and Enter included, stays the draft's.
    for typed in ["g", " ", "2"] {
        text(&mut app, typed);
    }
    key(&mut app, 8);
    text(&mut app, "3");
    key(&mut app, 13);
    assert_eq!(app.state.editor.text, "fix bug 3");
    assert!(app.state.question.is_some(), "nothing was answered");
    let panel = app.state.ask_panel.as_ref().unwrap();
    assert!(panel.entry().picked.is_empty() && !panel.entry().other);
    assert!(panel.entry().editor.text.is_empty());

    // After a pause, a digit is a direct choice and sends a single choice.
    app.state
        .ask_panel
        .as_mut()
        .unwrap()
        .guard
        .as_mut()
        .unwrap()
        .quiet();
    key(&mut app, 13);
    assert!(
        app.state.question.is_some(),
        "Enter with no focus chooses nothing"
    );
    text(&mut app, "2");
    assert!(app.state.question.is_none());
    assert_eq!(app.state.editor.text, "fix bug 3");
}

#[test]
fn an_arrow_lifts_the_question_guard_at_once() {
    let (directory, home) = (Directory::new(), Directory::new());
    let fixture = HttpFixture::new(vec![]);
    let mut app = app(&directory, &home, &fixture);
    text(&mut app, "draft");
    assert!(app.question_observer.begin(single()).unwrap());
    app.refresh_question();
    key(&mut app, 40);
    assert!(app.state.ask_panel.as_ref().unwrap().guard.is_none());
    assert_eq!(app.state.ask_panel.as_ref().unwrap().entry().row, 0);
    key(&mut app, 13);
    assert!(app.state.question.is_none());
    assert_eq!(app.state.editor.text, "draft");
}

#[test]
fn a_question_after_a_sent_or_idle_draft_has_no_typing_guard() {
    let (directory, home) = (Directory::new(), Directory::new());
    let fixture = HttpFixture::new(vec![]);
    let mut app = app(&directory, &home, &fixture);
    assert!(app.question_observer.begin(single()).unwrap());
    app.refresh_question();
    let panel = app.state.ask_panel.as_ref().unwrap();
    assert!(panel.guard.is_none());
    assert_eq!(panel.entry().row, 0, "the first choice is focused");
    key(&mut app, 13);
    assert!(app.state.question.is_none());
}

#[test]
fn a_permission_arriving_mid_typing_decides_nothing_until_the_keys_pause() {
    let (directory, home) = (Directory::new(), Directory::new());
    let fixture = HttpFixture::new(vec![]);
    let mut app = app(&directory, &home, &fixture);
    text(&mut app, "fix bug ");
    let access = Access {
        kind: AccessKind::Runs,
        target: "cargo test".into(),
        place: None,
        outside: true,
        base: None,
        identity: None,
    };
    app.permission_observer
        .begin("main-call", "bash", &access)
        .unwrap();
    app.poll().unwrap();
    assert!(app.state.permission.as_ref().unwrap().guard.is_some());
    text(&mut app, "2");
    key(&mut app, 13);
    assert_eq!(app.state.editor.text, "fix bug 2");
    assert!(app.permission_observer.pending().is_some());
    assert!(app.permission_observer.grants().is_empty());
    // Polling keeps the same request's guard.
    app.poll().unwrap();
    assert!(app.state.permission.as_ref().unwrap().guard.is_some());

    // After the pause the shortcut decides again; letters reach nothing.
    app.state
        .permission
        .as_mut()
        .unwrap()
        .guard
        .as_mut()
        .unwrap()
        .quiet();
    text(&mut app, "x");
    assert_eq!(app.state.editor.text, "fix bug 2");
    text(&mut app, "3");
    assert!(app.permission_observer.pending().is_none());
    assert!(app.permission_observer.grants().is_empty());
}

#[test]
fn an_arrow_lifts_the_permission_guard_and_moves_the_focus() {
    let (directory, home) = (Directory::new(), Directory::new());
    let fixture = HttpFixture::new(vec![]);
    let mut app = app(&directory, &home, &fixture);
    text(&mut app, "typing");
    let access = Access {
        kind: AccessKind::Runs,
        target: "cargo build".into(),
        place: None,
        outside: true,
        base: None,
        identity: None,
    };
    app.permission_observer
        .begin("main-call", "bash", &access)
        .unwrap();
    app.poll().unwrap();
    key(&mut app, 40);
    let shown = app.state.permission.as_ref().unwrap();
    assert!(shown.guard.is_none());
    assert_eq!(shown.focus, Some(0));
}
