use super::*;
use crate::tui::state::Kind;
use crate::tui::theme::{ACCENT, BODY, SELECTED_SURFACE};
use crate::{events::Event, json::Value};

fn work_read(state: &mut State, id: &str, work: &str, content: &str) {
    state.event(Event::ToolStarted {
        id: id.into(),
        name: "read".into(),
        arguments: Value::object([
            ("path", Value::string(format!("{id}.rs"))),
            ("work", Value::string(work)),
        ]),
    });
    state.event(Event::ToolFinished {
        id: id.into(),
        name: "read".into(),
        summary: "complete".into(),
        result: Value::object([
            ("content", Value::string(content)),
            ("lines_returned", Value::number(1)),
            ("offset", Value::number(1)),
        ]),
    });
}

fn ready(display: &mut Display, state: &State) {
    for _ in 0..100 {
        display.update(state, "fixture/model", "project", Instant::now());
        if !display.working {
            return;
        }
    }
    panic!("display did not finish its bounded layout");
}

fn read_at(display: &mut Display, text: &str) {
    let (block, row) = display
        .layout
        .blocks
        .iter()
        .enumerate()
        .find_map(|(block, lines)| {
            lines
                .iter()
                .position(|line| line.plain().contains(text))
                .map(|row| (block, row))
        })
        .expect("target line in expanded layout");
    let offset: usize = display.layout.blocks[..block].iter().map(Vec::len).sum();
    display.viewport.scroll(Scroll::Start);
    display.viewport.view(&display.layout.blocks, &[], 5);
    display
        .viewport
        .scroll(Scroll::Rows((offset + row) as isize));
    let slice = display.viewport.view(&display.layout.blocks, &[], 5);
    assert!(slice.lines[0].plain().contains(text));
}

#[test]
fn back_to_bottom_badge_is_centered_and_uses_the_blue_palette() {
    let line = back_to_bottom_line(40, false);
    assert_eq!(
        line.plain(),
        format!("{} ↓ Back to bottom · esc ", " ".repeat(8))
    );
    assert_eq!(line.spans[0].style, BODY);
    assert_eq!(line.spans[0].background, None);
    assert_eq!(line.spans[1].style, ACCENT);
    assert_eq!(line.spans[1].background, Some(SELECTED_SURFACE));
    assert!(
        line.paint(40)
            .contains(&format!("\x1b[{SELECTED_SURFACE}m"))
    );
}

#[test]
fn back_to_bottom_badge_fits_small_widths() {
    for alt_end in [false, true] {
        for width in 0..30 {
            let line = back_to_bottom_line(width, alt_end);
            assert!(super::super::text::cells(&line.plain()) <= width);
        }
    }
    assert_eq!(back_to_bottom_line(1, false).plain(), "↓");
    assert_eq!(back_to_bottom_line(7, false).plain(), " ↓ esc ");
    assert_eq!(back_to_bottom_line(11, true).plain(), " ↓ alt+end ");
}

#[test]
fn contextual_escape_actions_use_alt_end_in_the_badge() {
    let mut state = State::default();
    assert!(!back_to_bottom_uses_alt_end(&state));
    state.history.record("saved prompt");
    state.history.navigate(&mut state.editor, true);
    assert!(back_to_bottom_uses_alt_end(&state));
    state.history.cancel(&mut state.editor);
    state.queue.push("queued draft").unwrap();
    let slot = state.queue.slot(0).unwrap();
    state.queue.begin_edit(slot, &mut state.editor).unwrap();
    assert!(back_to_bottom_uses_alt_end(&state));
    state.queue.cancel_edit(&mut state.editor);
    state.suggestions.panel = true;
    assert!(back_to_bottom_uses_alt_end(&state));
    state.suggestions.panel = false;
    state.tool_details.expanded = true;
    assert!(!back_to_bottom_uses_alt_end(&state));
    assert!(
        back_to_bottom_line(40, true)
            .plain()
            .contains("Back to bottom · alt+end")
    );
}

#[test]
fn back_to_bottom_only_requests_the_end_while_reading() {
    let mut display = Display::new((40, 20));
    let blocks: Vec<_> = (0..20)
        .map(|index| vec![Line::new(&index.to_string(), BODY)])
        .collect();
    assert!(!display.back_to_bottom());
    display.viewport.view(&blocks, &[], 5);
    display.scroll(Scroll::Page(true));
    assert!(display.viewport.view(&blocks, &[], 5).reading);
    assert!(display.back_to_bottom());
    assert!(!display.viewport.view(&blocks, &[], 5).reading);
    assert!(!display.back_to_bottom());
}

#[test]
fn badge_appears_above_the_composer_only_while_reading() {
    let mut state = State::default();
    state.message(
        Kind::Assistant,
        &(0..60)
            .map(|index| format!("Row {index:02}  \n"))
            .collect::<String>(),
    );
    let mut display = Display::new((state.width, state.height));
    let now = Instant::now();
    let initial = display.update(&state, "fixture/model", "fixture directory", now);
    assert!(!initial.contains("Back to bottom"));
    display.scroll(Scroll::Page(true));
    let reading = display.update(&state, "fixture/model", "fixture directory", now);
    assert!(reading.contains("Back to bottom · esc"));
    state.history.record("saved prompt");
    state.history.navigate(&mut state.editor, true);
    let contextual = display.update(&state, "fixture/model", "fixture directory", now);
    assert!(contextual.contains("Back to bottom · alt+end"));
    assert!(display.back_to_bottom());
    let following = display.update(&state, "fixture/model", "fixture directory", now);
    assert!(!following.contains("Back to bottom"));
}

#[test]
fn short_conversations_do_not_show_a_badge_after_scroll_start() {
    let mut state = State::default();
    state.message(Kind::Assistant, "A short answer.");
    let mut display = Display::new((state.width, state.height));
    let now = Instant::now();
    display.update(&state, "fixture/model", "fixture directory", now);
    display.scroll(Scroll::Start);
    let output = display.update(&state, "fixture/model", "fixture directory", now);
    assert!(!output.contains("Back to bottom"));
    assert!(!display.back_to_bottom());
}

#[test]
fn collapsing_a_later_call_returns_reading_to_its_chapter_then_leaves_error_reading_alone() {
    let mut state = State::default();
    state.message(Kind::User, "Inspect and verify");
    state.begin_tool_turn();
    work_read(&mut state, "first", "Inspect parser", "first output\n");
    work_read(&mut state, "later", "Inspect parser", "later output\n");
    state.event(Event::ToolStarted {
        id: "error".into(),
        name: "bash".into(),
        arguments: Value::object([
            ("command", Value::string("cargo test parser")),
            ("work", Value::string("Inspect parser")),
        ]),
    });
    state.event(Event::ToolFinished {
        id: "error".into(),
        name: "bash".into(),
        summary: "error: parser test failed".into(),
        result: Value::object([
            ("stderr", Value::string("error: parser test failed\n")),
            ("exit_code", Value::number(101)),
        ]),
    });
    state.close_tools();
    state.finish_tool_turn();
    state.message(
        Kind::Assistant,
        &"Tail paragraph for scrolling.\n".repeat(80),
    );
    state.toggle_tools().unwrap();

    let mut display = Display::new((80, 24));
    ready(&mut display, &state);
    read_at(&mut display, "later output");
    state.toggle_tools().unwrap();
    ready(&mut display, &state);
    assert!(
        display
            .visible
            .iter()
            .take(3)
            .any(|line| line.plain().contains("Inspect parser")),
        "collapse should place the corresponding chapter at the reading position"
    );

    read_at(&mut display, "Exit 101");
    ready(&mut display, &state);
    assert!(
        display
            .visible
            .iter()
            .take(3)
            .any(|line| line.plain().contains("Exit 101")),
        "ordinary reading of a visible error must not jump back to the chapter"
    );

    state.toggle_tools().unwrap();
    ready(&mut display, &state);
    read_at(&mut display, "first output");
    state.toggle_tools().unwrap();
    ready(&mut display, &state);
    assert!(
        display
            .visible
            .iter()
            .take(3)
            .any(|line| line.plain().contains("Inspect parser")),
        "folding the first call's detail should keep its chapter header"
    );
}

#[test]
fn collapsing_the_latest_turn_preserves_older_chapters_and_unrelated_prose() {
    let mut state = State::default();
    state.message(Kind::User, "Earlier request");
    state.begin_tool_turn();
    work_read(&mut state, "earlier", "Earlier work", "earlier output\n");
    state.close_tools();
    state.finish_tool_turn();
    state.message(Kind::Assistant, "Unrelated prose anchor.");
    state.message(Kind::User, "Latest request");
    state.begin_tool_turn();
    work_read(&mut state, "latest", "Latest work", "latest output\n");
    state.close_tools();
    state.finish_tool_turn();
    state.message(Kind::Assistant, &"Long tail for scrolling.\n".repeat(80));

    let mut display = Display::new((80, 24));
    for target in ["Earlier work", "Unrelated prose anchor"] {
        state.toggle_tools().unwrap();
        ready(&mut display, &state);
        read_at(&mut display, target);
        state.toggle_tools().unwrap();
        ready(&mut display, &state);
        assert!(
            display
                .visible
                .iter()
                .take(3)
                .any(|line| line.plain().contains(target)),
            "collapsing the latest turn moved an unrelated reading anchor: {target}"
        );
    }
}

#[test]
fn collapsing_while_following_keeps_the_tail_in_view() {
    let mut state = State::default();
    state.message(Kind::User, "Read a file");
    state.begin_tool_turn();
    work_read(&mut state, "file", "Inspect file", "contents\n");
    state.close_tools();
    state.finish_tool_turn();
    state.message(Kind::Assistant, &"Tail output.\n".repeat(80));
    state.toggle_tools().unwrap();
    let mut display = Display::new((80, 24));
    ready(&mut display, &state);
    assert!(display.viewport.following());

    state.toggle_tools().unwrap();
    ready(&mut display, &state);
    assert!(display.viewport.following());
    assert!(
        display
            .visible
            .iter()
            .any(|line| line.plain().contains("Tail output."))
    );
}
