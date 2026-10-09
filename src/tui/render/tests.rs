use super::*;
use crate::tui::{
    state::{Kind, State},
    view,
};
use crate::{events::Event, json::Value};
mod screen;
use screen::Screen;

fn update(renderer: &mut Renderer, screen: &mut Screen, state: &mut State) -> String {
    screen.enter();
    let output = renderer.update(
        view::frame(state, "fixture/model", "fixture directory"),
        state.width,
        state.height,
    );
    assert!(!output.contains('\n') && !output.contains("\x1b[3J"));
    screen.feed(&output);
    output
}

fn start(state: &mut State, id: &str) {
    state.event(Event::ToolStarted {
        id: id.into(),
        name: "bash".into(),
        arguments: Value::object([("command", Value::string(id))]),
    });
}
fn finish(state: &mut State, id: &str, output: &str) {
    state.event(Event::ToolFinished {
        id: id.into(),
        name: "bash".into(),
        summary: "exit 0".into(),
        result: Value::object([
            ("exit_code", Value::number(0)),
            ("stdout", Value::string(output)),
        ]),
    });
}

#[test]
fn fullscreen_hides_and_restores_the_shell_and_keeps_the_composer_at_the_bottom() {
    let mut state = State::default();
    let mut screen = Screen::new(state.width, state.height);
    screen.feed("existing shell output\r\nPS> ");
    let shell = screen.visible();
    let position = (screen.x, screen.y);
    let mut renderer = Renderer::new();
    state.message(Kind::User, "inspect this project");
    state.message(Kind::Assistant, "A readable **answer**.");
    update(&mut renderer, &mut screen, &mut state);
    assert!(!screen.visible().contains("existing shell output"));
    assert!(screen.visible().contains("A readable answer."));
    assert_eq!(screen.y, state.height - 3);
    state.editor.insert("next");
    let edit = update(&mut renderer, &mut screen, &mut state);
    assert!(!edit.contains("readable"));
    assert!(screen.visible().contains("› next"));
    assert_eq!((screen.x, screen.y), (6, state.height - 3));
    screen.feed(crate::tui::terminal::LEAVE);
    assert_eq!(screen.visible(), shell);
    assert_eq!((screen.x, screen.y), position);
    screen.feed("Jecode closed\r\nResume: jecode resume fixture-id\r\n");
    assert!(!screen.visible().contains("A readable answer."));
    assert!(screen.visible().contains("Jecode closed"));
}

#[test]
fn streaming_finalization_updates_the_same_visible_message_and_cursor() {
    let mut state = State::default();
    let mut screen = Screen::new(state.width, state.height);
    let mut renderer = Renderer::new();
    state.editor.insert("first\nsecond🙂");
    state.message(Kind::User, "question");
    for text in ["A", "A\n\nB"] {
        state.event(Event::Streaming { text: text.into() });
        update(&mut renderer, &mut screen, &mut state);
    }
    let before = screen.visible();
    state.event(Event::Message {
        text: "A\n\nB".into(),
    });
    update(&mut renderer, &mut screen, &mut state);
    assert_eq!(screen.visible(), before);
    assert_eq!(state.editor.text, "first\nsecond🙂");
    assert!(screen.history.is_empty());
    assert!(update(&mut renderer, &mut screen, &mut state).is_empty());
}

#[test]
fn out_of_order_tool_results_remain_in_source_order_and_never_duplicate_cards() {
    let mut state = State::default();
    start(&mut state, "first");
    start(&mut state, "second");
    finish(&mut state, "second", "second output");
    finish(&mut state, "first", "first output");
    state.close_tools();
    state.select_tool(Some(0));
    state.toggle_tool();
    state.select_tool(None);
    let mut screen = Screen::new(state.width, state.height);
    let mut renderer = Renderer::new();
    update(&mut renderer, &mut screen, &mut state);
    let visible = screen.visible();
    assert!(visible.find("first output").unwrap() < visible.find("second output").unwrap());
    assert_eq!(visible.matches("first output").count(), 1);
    assert!(visible.contains("└─ bash  second"));
    assert!(screen.history.is_empty());
}

#[test]
fn a_new_conversation_replaces_only_the_owned_display() {
    let mut state = State::default();
    let mut screen = Screen::new(state.width, state.height);
    screen.feed("shell history\r\n");
    let shell = screen.visible();
    let mut renderer = Renderer::new();
    state.message(Kind::Assistant, "Previous conversation.");
    update(&mut renderer, &mut screen, &mut state);
    state.clear();
    state.message(Kind::User, "New conversation.");
    update(&mut renderer, &mut screen, &mut state);
    assert!(!screen.visible().contains("Previous conversation."));
    assert!(screen.visible().contains("New conversation."));
    screen.feed(crate::tui::terminal::LEAVE);
    assert_eq!(screen.visible(), shell);
}

#[test]
fn full_width_rows_and_the_bottom_right_cell_never_scroll_or_leak_into_shell_history() {
    for (width, height) in [(2, 2), (12, 4), (80, 24)] {
        let mut screen = Screen::new(width, height);
        screen.feed("S");
        let shell = screen.visible();
        screen.enter();
        let rows: Vec<_> = (0..height)
            .map(|row| {
                char::from(b'A' + (row % 26) as u8)
                    .to_string()
                    .repeat(width)
            })
            .collect();
        let mut frame = view::Frame {
            header: vec![],
            history: vec![],
            live: rows.iter().map(|row| Line::new(row, "0")).collect(),
            cursor: Some((height - 1, width - 1)),
            composer: height - 1,
        };
        let mut renderer = Renderer::new();
        screen.feed(&renderer.paint(&frame, width, height));
        assert_eq!(screen.visible(), rows.join("\n"));
        assert_eq!((screen.x, screen.y), (width - 1, height - 1));
        assert!(screen.history.is_empty());
        frame.live[height - 1] = Line::new("X", "0");
        screen.feed(&renderer.paint(&frame, width, height));
        assert_eq!(screen.rows[0].iter().collect::<String>(), rows[0]);
        assert_eq!(
            screen.rows[height - 1].iter().collect::<String>(),
            format!("X{}", " ".repeat(width - 1))
        );
        screen.feed(crate::tui::terminal::LEAVE);
        assert_eq!(screen.visible(), shell);
        assert!(screen.history.is_empty());
    }
}

mod composer_tests;
mod delete_tests;
mod feedback_tests;
mod flow_tests;
mod input_tests;
mod markdown_tests;
mod resize_tests;
mod spacing_tests;
