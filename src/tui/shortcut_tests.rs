use super::*;
use crate::{
    backend::openrouter::OpenRouter,
    config::{Settings, Store},
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

#[test]
fn control_o_toggles_completed_tool_details_without_editing_the_draft() {
    let directory = Directory::new();
    let mut app = app(&directory);
    app.state.editor.insert("draft kept");
    app.state.begin_tool_turn();
    app.state.event(crate::events::Event::ToolStarted {
        id: "command".into(),
        name: "bash".into(),
        arguments: crate::json::Value::object([(
            "command",
            crate::json::Value::string("cargo build"),
        )]),
    });
    app.state.event(crate::events::Event::ToolFinished {
        id: "command".into(),
        name: "bash".into(),
        summary: "exit 0".into(),
        result: crate::json::Value::object([(
            "stdout",
            crate::json::Value::string("full tool output"),
        )]),
    });
    app.state.finish_tool_turn();
    for expanded in [true, false, true] {
        assert!(!control(&mut app, 79));
        let state::Item::Tool { presentation, .. } = app.state.items.last().unwrap() else {
            panic!("tool is present");
        };
        assert_eq!(presentation.expanded, expanded);
        assert_eq!(app.state.editor.text, "draft kept");
    }
    assert!(!app.state.tool_details.active);
    assert_eq!(app.state.editor.text, "draft kept");
    app.state.editor.replace("Follow-up prompt".into());
    assert!(
        !app.terminal_event(
            terminal::Input::Key(terminal::Key {
                code: 13,
                modifiers: 0,
                character: 0,
            }),
            &mut Decoder::default(),
        )
        .unwrap()
    );
    assert!(app.worker.is_some());
    assert!(app.state.editor.text.is_empty());
    assert!(!app.state.tool_details.expanded);
}
