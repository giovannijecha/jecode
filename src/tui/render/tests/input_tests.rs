use super::*;
use crate::tui::{
    App, Decoded,
    keys::Decoder,
    terminal::{Input, Key},
};
use crate::{agent::Agent, openrouter::OpenRouter, test_support::Directory, tools::Tools};
use std::time::Instant;

fn app(directory: &Directory) -> App {
    let mut app = App::new(
        Agent::new(
            OpenRouter::fixture("http://127.0.0.1:1".into()),
            Tools::new(directory.path()).unwrap(),
        ),
        crate::tui::tests::config(directory),
        None,
    );
    app.state.message(
        Kind::Assistant,
        &(0..100)
            .map(|i| format!("Row {i:03}\n"))
            .collect::<String>(),
    );
    app.state.history.record("first prompt");
    app.state.history.record("second prompt");
    app.state.editor.insert("kept draft");
    app.state.editor.cursor = 3;
    app
}

fn paint(app: &mut App, screen: &mut Screen) {
    screen.enter();
    screen.feed(&app.renderer.update(
        &app.state,
        "fixture/model",
        "fixture directory",
        Instant::now(),
    ));
}

fn key(app: &mut App, code: u16, modifiers: u8) {
    assert!(
        !app.input(Decoded::Key(Key {
            code,
            modifiers,
            character: 0
        }))
        .unwrap()
    );
}

#[test]
fn pages_and_wheel_scroll_the_conversation_while_control_keys_browse_prompts() {
    let directory = Directory::new();
    let mut app = app(&directory);
    let draft = app.state.editor.clone();
    let history = app.state.history.snapshot();
    let mut screen = Screen::new(80, 24);
    paint(&mut app, &mut screen);
    let latest = screen.visible();
    key(&mut app, 33, 0);
    paint(&mut app, &mut screen);
    assert!(screen.visible().contains("Back to bottom"));
    assert_eq!(app.state.editor, draft);
    assert_eq!(app.state.history.snapshot(), history);
    key(&mut app, 38, 0);
    assert_eq!(app.state.editor, draft);
    key(&mut app, 80, 4);
    assert_eq!(app.state.editor.text, "second prompt");
    key(&mut app, 78, 4);
    assert_eq!(app.state.editor, draft);
    key(&mut app, 34, 0);
    paint(&mut app, &mut screen);
    assert_eq!(screen.visible(), latest);
    let mut decoder = Decoder::default();
    app.terminal_event(Input::Scroll(-3), &mut decoder).unwrap();
    paint(&mut app, &mut screen);
    assert!(screen.visible().contains("Back to bottom"));
    assert_eq!(app.state.editor, draft);
    assert_eq!(app.state.history.snapshot(), history);
    for event in decoder.bytes(b"\x1b[<65;10;5M") {
        app.input(event).unwrap();
    }
    paint(&mut app, &mut screen);
    assert_eq!(screen.visible(), latest);
}

#[test]
fn streaming_and_panels_keep_the_reading_position_and_new_resets_it() {
    let directory = Directory::new();
    let mut app = app(&directory);
    let mut screen = Screen::new(80, 24);
    paint(&mut app, &mut screen);
    key(&mut app, 33, 0);
    paint(&mut app, &mut screen);
    let top = screen.rows[0].clone();
    app.state.event(Event::Streaming {
        text: "New streamed tail\n".repeat(40),
    });
    paint(&mut app, &mut screen);
    assert_eq!(screen.rows[0], top);
    app.help();
    paint(&mut app, &mut screen);
    assert!(screen.visible().contains("Commands and controls"));
    key(&mut app, 27, 0);
    paint(&mut app, &mut screen);
    assert_eq!(screen.rows[0], top);
    key(&mut app, 35, 1);
    paint(&mut app, &mut screen);
    assert!(!screen.visible().contains("Back to bottom"));
    assert!(screen.visible().contains("New streamed tail"));
    app.state.clear();
    app.state.message(Kind::Assistant, "New conversation");
    paint(&mut app, &mut screen);
    assert!(!screen.visible().contains("Row 0") && !screen.visible().contains("New streamed tail"));
    assert!(screen.visible().contains("New conversation"));
    assert_eq!(app.state.editor.text, "kept draft");
}

#[test]
fn escape_returns_to_bottom_without_stopping_work_and_panels_keep_escape_priority() {
    use crate::test_support::{HttpFixture, Response};
    use std::time::Duration;
    let directory = Directory::new();
    let fixture = HttpFixture::streaming(vec![Response::Stream(vec![
        (Duration::ZERO, "data: {\"choices\":[{\"delta\":{\"reasoning\":\"fixture\"},\"finish_reason\":null}]}\n\n".into()),
        (Duration::from_millis(350), "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n".into()),
    ])]);
    let mut app = App::new(
        Agent::new(
            OpenRouter::fixture(fixture.endpoint.clone()),
            Tools::new(directory.path()).unwrap(),
        ),
        crate::tui::tests::config(&directory),
        None,
    );
    app.state
        .message(Kind::Assistant, &"conversation row\n".repeat(80));
    app.state.editor.insert("active request");
    app.submit().unwrap();
    app.state.editor.insert("kept draft");
    app.state.editor.cursor = 2;
    let draft = app.state.editor.clone();
    let deadline = Instant::now() + Duration::from_secs(6);
    while app.state.activity.as_ref().unwrap().label != "Thinking" {
        app.poll().unwrap();
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(5));
    }
    let mut screen = Screen::new(80, 24);
    paint(&mut app, &mut screen);
    key(&mut app, 33, 0);
    paint(&mut app, &mut screen);
    assert!(screen.visible().contains("Back to bottom · esc"));
    assert!(screen.visible().contains("Esc bottom"));
    key(&mut app, 27, 0);
    paint(&mut app, &mut screen);
    assert!(!screen.visible().contains("Back to bottom"));
    assert!(!app.state.activity.as_ref().unwrap().stopping);
    assert_eq!(app.state.editor, draft);
    key(&mut app, 33, 0);
    paint(&mut app, &mut screen);
    app.open_drafts();
    paint(&mut app, &mut screen);
    assert!(screen.visible().contains("Back to bottom · Alt+End"));
    key(&mut app, 27, 0);
    paint(&mut app, &mut screen);
    assert!(!app.draft_menu_open());
    assert!(screen.visible().contains("Back to bottom · esc"));
    assert!(!app.state.activity.as_ref().unwrap().stopping);
    app.open_drafts();
    key(&mut app, 35, 1);
    paint(&mut app, &mut screen);
    assert!(app.draft_menu_open());
    assert!(!screen.visible().contains("Back to bottom"));
    key(&mut app, 27, 0);
    key(&mut app, 67, 4);
    while app.worker.is_some() {
        app.poll().unwrap();
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(app.state.editor, draft);
    assert_eq!(fixture.finish().len(), 1);
}
