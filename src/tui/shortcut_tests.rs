use super::*;
use crate::{
    config::{Settings, Store},
    openrouter::OpenRouter,
    test_support::{Directory, HttpFixture},
    tools::Tools,
};

fn app(directory: &Directory) -> App {
    let fixture = HttpFixture::new(vec![]);
    App::new(
        Agent::new(
            OpenRouter::fixture(fixture.endpoint.clone()),
            Tools::new(directory.path()).unwrap(),
        ),
        SessionConfig {
            store: Store::new(directory.path().join("config")),
            settings: Settings::new("isolated-fixture-key".into(), "fixture/model".into()).unwrap(),
            bash: crate::tools::find_bash().unwrap(),
        },
        None,
    )
}

fn control(app: &mut App, code: u16) -> bool {
    app.terminal_event(
        terminal::Input::Key(terminal::Key {
            code,
            modifiers: 4,
            character: 0,
        }),
        &mut Decoder::default(),
    )
    .unwrap()
}

#[test]
fn native_paste_inserts_unicode_and_multiline_text_without_submitting() {
    let directory = Directory::new();
    let mut app = app(&directory);
    assert!(
        !app.terminal_event(
            terminal::Input::Paste("kept🙂\r\nnext".encode_utf16().collect()),
            &mut Decoder::default()
        )
        .unwrap()
    );
    assert_eq!(app.state.editor.text, "kept🙂\nnext");
    assert!(app.worker.is_none());
    assert!(!control(&mut app, 67));
}

#[test]
fn control_c_never_exits_and_control_q_exits_with_or_without_a_draft() {
    let directory = Directory::new();
    let mut app = app(&directory);
    assert!(!control(&mut app, 67));
    app.state.editor.insert("draft");
    assert!(!control(&mut app, 67));
    assert!(app.state.editor.text.is_empty());
    assert!(!control(&mut app, 67));
    assert!(control(&mut app, 81));
    app.state.editor.insert("saved draft");
    assert!(control(&mut app, 81));
    assert_eq!(app.state.editor.text, "saved draft");
}

#[test]
fn control_c_closes_a_menu_without_exiting_or_clearing_the_draft() {
    let directory = Directory::new();
    let mut app = app(&directory);
    app.state.editor.insert("draft kept");
    app.dispatch("/effort".into()).unwrap();
    assert!(app.state.selector.is_some());
    assert!(!control(&mut app, 67));
    assert!(app.state.selector.is_none());
    assert_eq!(app.state.editor.text, "draft kept");
    assert!(control(&mut app, 81));
}
