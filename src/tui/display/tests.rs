use super::*;
use crate::tui::state::Kind;
use crate::tui::theme::{ACCENT, BODY, DIM, LIT, SELECTED_SURFACE};
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
fn the_reading_row_counts_the_rows_below_and_ends_its_key_two_cells_from_the_edge() {
    let line = view::reading_row(40, 16, false, false);
    let plain = line.plain();
    assert_eq!(super::super::text::cells(&plain), 38);
    assert!(plain.starts_with("↓ 16 rows below "));
    assert!(plain.ends_with(" Esc  back to bottom"));
    assert_eq!(line.spans[0].style, DIM.bold());
    assert!(
        line.spans
            .iter()
            .any(|span| { span.text.contains("Esc") && span.background == Some(SELECTED_SURFACE) })
    );
    assert!(
        view::reading_row(40, 1, false, false)
            .plain()
            .starts_with("↓ 1 row below ")
    );

    let fresh = view::reading_row(60, 3, true, true);
    assert_eq!(fresh.spans[0].style, ACCENT.bold());
    assert!(fresh.plain().starts_with("↓ 3 rows below · new output "));
    assert!(fresh.plain().ends_with(" Alt+End  back to bottom"));
    assert!(
        fresh
            .spans
            .iter()
            .any(|span| span.text == "new output" && span.style == LIT)
    );
}

#[test]
fn the_reading_row_drops_its_key_first_and_fits_small_widths() {
    for alt_end in [false, true] {
        for width in 3..60 {
            let line = view::reading_row(width, 120, true, alt_end);
            assert!(super::super::text::cells(&line.plain()) <= width - 2);
        }
    }
    let narrow = view::reading_row(32, 16, true, false).plain();
    assert!(narrow.starts_with("↓ 16 rows below · new output"));
    assert!(!narrow.contains("Esc"));
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
    state.quitting = Some(Vec::new());
    assert!(back_to_bottom_uses_alt_end(&state));
    state.quitting = None;
    state.tool_details.expanded = true;
    assert!(!back_to_bottom_uses_alt_end(&state));
    assert!(
        view::reading_row(60, 1, false, true)
            .plain()
            .contains("Alt+End  back to bottom")
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
    assert!(!initial.contains("back to bottom"));
    display.scroll(Scroll::Page(true));
    let reading = display.update(&state, "fixture/model", "fixture directory", now);
    assert!(reading.contains(" Esc ") && reading.contains(" back to bottom"));
    state.history.record("saved prompt");
    state.history.navigate(&mut state.editor, true);
    let contextual = display.update(&state, "fixture/model", "fixture directory", now);
    assert!(contextual.contains(" Alt+End ") && contextual.contains(" back to bottom"));
    assert!(display.back_to_bottom());
    let following = display.update(&state, "fixture/model", "fixture directory", now);
    assert!(!following.contains("back to bottom"));
}

#[test]
fn the_reading_row_marks_new_output_but_not_a_reflow() {
    let mut state = State::default();
    state.message(
        Kind::Assistant,
        &(0..60)
            .map(|index| format!("Row {index:02}  \n"))
            .collect::<String>(),
    );
    let mut display = Display::new((state.width, state.height));
    let now = Instant::now();
    display.update(&state, "fixture/model", "fixture directory", now);
    display.scroll(Scroll::Page(true));
    let reading = display.update(&state, "fixture/model", "fixture directory", now);
    assert!(reading.contains(" rows below"));
    assert!(!reading.contains("new output"));
    assert!(display.below > 0);
    let below = display.below;

    state.width += 7;
    let reflowed = display.update(&state, "fixture/model", "fixture directory", now);
    assert!(!reflowed.contains("new output"));
    assert!(!display.fresh);

    state.message(Kind::Assistant, "A later answer.");
    let fresh = display.update(&state, "fixture/model", "fixture directory", now);
    assert!(fresh.contains("new output"));
    assert!(display.fresh && display.below > below);

    assert!(display.back_to_bottom());
    display.update(&state, "fixture/model", "fixture directory", now);
    display.scroll(Scroll::Page(true));
    let again = display.update(&state, "fixture/model", "fixture directory", now);
    assert!(!again.contains("new output"));
}

#[test]
fn expanding_existing_tools_does_not_mark_new_output_while_reading() {
    let mut state = State::default();
    state.message(Kind::User, "Read a file");
    state.begin_tool_turn();
    work_read(
        &mut state,
        "file",
        "Inspect file",
        &"source line\n".repeat(20),
    );
    state.close_tools();
    state.finish_tool_turn();
    state.message(Kind::Assistant, &"Later answer.\n".repeat(80));

    let mut display = Display::new((state.width, state.height));
    ready(&mut display, &state);
    display.scroll(Scroll::Page(true));
    ready(&mut display, &state);
    assert!(!display.fresh);

    for expanded in [true, false, true] {
        assert_eq!(state.toggle_tools().map(|(value, _)| value), Some(expanded));
        ready(&mut display, &state);
        assert!(!display.fresh, "presentation reflow is not new output");
        assert!(!display.viewport.following());
    }
}

#[test]
fn streaming_more_text_on_the_same_row_marks_new_output_while_reading() {
    let mut state = State::default();
    state.message(Kind::Assistant, &"Earlier answer.\n".repeat(80));
    state.event(Event::Streaming {
        text: "first".into(),
    });
    let mut display = Display::new((state.width, state.height));
    ready(&mut display, &state);
    display.scroll(Scroll::Page(true));
    ready(&mut display, &state);
    let below = display.below;
    assert!(!display.fresh);

    state.event(Event::Streaming {
        text: "first more".into(),
    });
    ready(&mut display, &state);
    assert_eq!(display.below, below);
    assert!(display.fresh);
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
    assert!(!output.contains("back to bottom"));
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

    read_at(&mut display, "exit 101");
    ready(&mut display, &state);
    assert!(
        display
            .visible
            .iter()
            .take(3)
            .any(|line| line.plain().contains("exit 101")),
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
    work_read(
        &mut state,
        "earlier-2",
        "Earlier work",
        "other earlier output\n",
    );
    state.close_tools();
    state.finish_tool_turn();
    state.message(Kind::Assistant, "Unrelated prose anchor.");
    state.message(Kind::User, "Latest request");
    state.begin_tool_turn();
    work_read(&mut state, "latest", "Latest work", "latest output\n");
    work_read(
        &mut state,
        "latest-2",
        "Latest work",
        "other latest output\n",
    );
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

#[test]
fn singleton_preview_anchor_stays_on_its_visible_source_when_collapsed() {
    let mut state = State::default();
    state.begin_tool_turn();
    state.event(Event::ToolStarted {
        id: "single".into(),
        name: "bash".into(),
        arguments: Value::object([("command", Value::string("printf example"))]),
    });
    state.event(Event::ToolFinished {
        id: "single".into(),
        name: "bash".into(),
        summary: "Done".into(),
        result: Value::object([
            ("exit_code", Value::number(0)),
            ("stdout", Value::string("singleton preview anchor\n")),
        ]),
    });
    state.close_tools();
    state.finish_tool_turn();
    state.message(Kind::Assistant, &"Long tail for scrolling.\n".repeat(80));
    state.toggle_tools().unwrap();
    let mut display = Display::new((80, 24));
    ready(&mut display, &state);
    read_at(&mut display, "singleton preview anchor");
    state.toggle_tools().unwrap();
    ready(&mut display, &state);
    assert!(
        display
            .visible
            .first()
            .is_some_and(|row| row.plain().contains("singleton preview anchor"))
    );
}
