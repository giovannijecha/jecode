use super::*;
use crate::{events::Event, json::Value, tui::theme::MUTED};

#[test]
fn a_new_conversation_discards_the_previous_display_source_and_cache() {
    let mut state = State::default();
    state.message(Kind::Assistant, "Previous **answer**.");
    state.clear();
    assert!(state.items.is_empty());
    state.message(Kind::User, "New request.");
    let mut layout = Layout::default();
    layout.prepare(&state, 79, 64);
    let text = layout
        .blocks
        .iter()
        .flatten()
        .map(Line::plain)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(!text.contains("Previous answer."));
    assert!(text.contains("New request."));
    assert_eq!(text.matches("Jecode").count(), 1);
}

#[test]
fn layout_work_is_bounded_and_finalization_appends_the_original_stream_only_once() {
    let mut state = State::default();
    for number in 0..150 {
        state.message(Kind::Assistant, &format!("Block {number}."));
    }
    state.event(Event::Streaming {
        text: "Live response.".into(),
    });
    let mut layout = Layout::default();
    layout.prepare(&state, 39, 64);
    assert_eq!(layout.blocks.len(), 64);
    assert!(!layout.ready(&state));
    layout.prepare(&state, 39, 64);
    layout.prepare(&state, 39, 64);
    assert!(layout.ready(&state));
    assert_eq!(layout.blocks.len(), 151);
    assert!(
        layout
            .pending(&state)
            .iter()
            .any(|line| line.plain() == "● Live response.")
    );
    state.event(Event::Message {
        text: "Live response.".into(),
    });
    layout.prepare(&state, 39, 64);
    assert!(layout.pending(&state).is_empty());
    assert_eq!(
        layout
            .blocks
            .iter()
            .flatten()
            .filter(|line| line.plain() == "● Live response.")
            .count(),
        1
    );
}

#[test]
fn a_finished_tool_stays_live_until_bookkeeping_is_settled_without_visual_change() {
    let mut state = State::default();
    state.event(Event::ToolStarted {
        id: "one".into(),
        name: "read".into(),
        arguments: Value::object([("path", Value::string("fixture.rs"))]),
    });
    state.event(Event::ToolFinished {
        id: "one".into(),
        name: "read".into(),
        summary: "complete".into(),
        result: Value::object([("content", Value::string("fixture content"))]),
    });
    let mut layout = Layout::default();
    layout.prepare(&state, 39, 64);
    assert_eq!(layout.blocks.len(), 1);
    assert!(
        layout.pending(&state)[0]
            .plain()
            .starts_with("● Read(fixture.rs)")
    );
    let pending = layout.pending(&state).to_vec();
    state.close_tools();
    layout.prepare(&state, 39, 64);
    assert_eq!(layout.blocks.len(), 2);
    assert_eq!(layout.blocks[1], pending);
    assert!(matches!(
        state.items[0],
        Item::Tool {
            last: Some(true),
            ..
        }
    ));
}

#[test]
fn a_width_change_rebuilds_rows_from_source_and_a_height_change_keeps_the_same_layout() {
    let mut state = State::default();
    state.message(Kind::Assistant, "one two three four five six seven eight");
    let mut layout = Layout::default();
    layout.prepare(&state, 79, 64);
    let wide = layout.blocks.clone();
    state.height = 3;
    layout.prepare(&state, 79, 64);
    assert_eq!(layout.blocks, wide);
    layout.prepare(&state, 19, 64);
    assert!(layout.blocks[1].len() > wide[1].len());
    layout.prepare(&state, 79, 64);
    assert_eq!(layout.blocks, wide);
}

#[test]
fn retained_invisible_assistant_keeps_a_single_separator_between_visible_messages() {
    let mut state = State::default();
    state.message(Kind::Assistant, "First");
    state.items.push(Item::Text {
        kind: Kind::Assistant,
        text: " \n ".into(),
    });
    state.message(Kind::Assistant, "Second");
    let mut layout = Layout::default();
    layout.prepare(&state, 40, 64);
    assert!(layout.ready(&state));
    assert_eq!(
        layout.blocks[1].iter().map(Line::plain).collect::<Vec<_>>(),
        ["● First"]
    );
    assert!(layout.blocks[2].is_empty());
    assert_eq!(
        layout.blocks[3].iter().map(Line::plain).collect::<Vec<_>>(),
        ["", "● Second"]
    );
    assert_eq!(layout.blocks[1..], state.rows());
}

#[test]
fn a_single_long_markdown_block_yields_between_passes_and_keeps_fence_state() {
    let mut state = State::default();
    let text = format!(
        "```rust\n/*\n{}*/\nlet value = 1;\n```\nFinal paragraph.",
        "comment line\n".repeat(600)
    );
    state.message(Kind::Assistant, &text);
    let mut layout = Layout::default();
    layout.prepare(&state, 39, 64);
    assert_eq!(
        layout.blocks.len(),
        1,
        "only the header is complete after the first bounded pass"
    );
    assert!(!layout.ready(&state));
    for _ in 0..10 {
        layout.prepare(&state, 39, 64);
        if layout.ready(&state) {
            break;
        }
    }
    assert!(layout.ready(&state));
    let body = &layout.blocks[1];
    assert_eq!(
        body.iter()
            .filter(|line| line.plain().trim() == "comment line")
            .count(),
        600
    );
    assert_eq!(body[0].plain(), "●  rust");
    // After the panel's one-cell padding, the comment keeps its colour.
    assert!(body[500].spans[2..].iter().all(|span| span.style == MUTED));
    let code = body
        .iter()
        .find(|line| line.plain().trim() == "let value = 1;")
        .unwrap();
    assert!(
        code.paint(39)
            .contains(&crate::tui::theme::KEYWORD.to_string())
    );
    assert_eq!(body.last().unwrap().plain(), "  Final paragraph.");
}

#[test]
fn toggling_tool_details_keeps_the_settled_transcript_ready() {
    let mut state = State::default();
    for index in 0..5_000 {
        state.message(Kind::Assistant, &format!("Message {index}"));
    }
    state.begin_tool_turn();
    state.event(Event::ToolStarted {
        id: "tool".into(),
        name: "read".into(),
        arguments: Value::object([("path", Value::string("fixture.rs"))]),
    });
    state.event(Event::ToolFinished {
        id: "tool".into(),
        name: "read".into(),
        summary: "complete".into(),
        result: Value::object([("content", Value::string("contents"))]),
    });
    state.close_tools();
    state.finish_tool_turn();
    let tool = state.items.len() - 1;
    let mut layout = Layout::default();
    for _ in 0..100 {
        layout.prepare(&state, 60, 64);
        if layout.ready(&state) {
            break;
        }
    }
    assert!(layout.ready(&state));
    let prior = layout.blocks[..tool + 1].to_vec();
    let compact_tool = layout.blocks[tool + 1].clone();
    state.toggle_tools();
    layout.prepare(&state, 60, 64);
    assert!(layout.ready(&state));
    assert_eq!(layout.blocks[..tool + 1], prior);
    assert!(state.tool_details.expanded);
    assert_ne!(layout.blocks[tool + 1], compact_tool);
    state.toggle_tools();
    layout.prepare(&state, 60, 64);
    assert!(layout.ready(&state));
    assert!(!state.tool_details.expanded);
    assert_eq!(layout.blocks[..tool + 1], prior);
    assert_eq!(layout.blocks[tool + 1], compact_tool);
}

fn read(state: &mut State, id: &str, path: &str) {
    state.event(Event::ToolStarted {
        id: id.into(),
        name: "read".into(),
        arguments: Value::object([("path", Value::string(path))]),
    });
}

fn finish_read(state: &mut State, id: &str) {
    state.event(Event::ToolFinished {
        id: id.into(),
        name: "read".into(),
        summary: "complete".into(),
        result: Value::object([
            ("content", Value::string("1: x\n")),
            ("lines_returned", Value::number(1)),
            ("offset", Value::number(1)),
        ]),
    });
}

#[test]
fn simultaneous_read_completions_keep_the_earlier_layout_cached() {
    for calls in [3, 100] {
        let mut state = State::default();
        for index in 0..150 {
            state.message(Kind::Assistant, &format!("Message {index}"));
        }
        state.begin_tool_turn();
        for index in 0..calls {
            read(&mut state, &index.to_string(), &format!("src/{index}.rs"));
        }
        let mut layout = Layout::default();
        assert!(shown(&mut layout, &state).contains("Message 149"));
        let prior = layout.blocks.clone();
        for index in 0..calls {
            finish_read(&mut state, &index.to_string());
        }
        assert!(state.tool_revision >= 2);
        layout.prepare(&state, 80, 1);
        assert!(
            layout.blocks.len() >= prior.len(),
            "settled prefix was rebuilt: {} cached blocks became {}",
            prior.len(),
            layout.blocks.len()
        );
        assert_eq!(&layout.blocks[..prior.len()], prior.as_slice());
        state.close_tools();
        let output = shown(&mut layout, &state);
        assert!(output.contains(&format!("● Activity  {calls} calls")));
        assert!(output.contains(&format!("{calls} files read")));
        assert_eq!(output.matches("● Activity  ").count(), 1);
    }
}

#[test]
fn missed_read_group_and_tool_toggles_redraw_from_the_earliest_change() {
    let mut state = State::default();
    for index in 0..150 {
        state.message(Kind::Assistant, &format!("Message {index}"));
    }
    state.begin_tool_turn();
    for (id, path) in [("a", "src/a.rs"), ("b", "src/b.rs"), ("c", "src/c.rs")] {
        read(&mut state, id, path);
    }
    let mut layout = Layout::default();
    shown(&mut layout, &state);
    let prior = layout.blocks.clone();
    for id in ["a", "b", "c"] {
        finish_read(&mut state, id);
    }
    state.close_tools();
    state.finish_tool_turn();
    state.toggle_tools().expect("completed read group");
    state.begin_tool_turn();
    assert!(!state.tool_details.expanded);
    layout.prepare(&state, 80, 64);
    assert!(
        layout.blocks.len() >= prior.len(),
        "settled prefix was rebuilt"
    );
    assert_eq!(&layout.blocks[..prior.len()], prior.as_slice());
    let output = shown(&mut layout, &state);
    assert!(output.contains("● Activity  3 calls"));
    assert!(output.contains("3 files read"));
    assert_eq!(output.matches("● Activity  ").count(), 1);
}

#[test]
fn a_layout_older_than_the_bounded_tool_history_still_matches_a_fresh_layout() {
    let mut state = State::default();
    for index in 0..150 {
        state.message(Kind::Assistant, &format!("Message {index}"));
    }
    state.begin_tool_turn();
    read(&mut state, "a", "src/a.rs");
    finish_read(&mut state, "a");
    state.close_tools();
    state.finish_tool_turn();
    let mut layout = Layout::default();
    shown(&mut layout, &state);
    for index in 0..66 {
        state.message(Kind::User, "Next read group");
        state.begin_tool_turn();
        for suffix in ["a", "b"] {
            let id = format!("{index}-{suffix}");
            read(&mut state, &id, &format!("src/{id}.rs"));
            finish_read(&mut state, &id);
        }
        state.close_tools();
        state.finish_tool_turn();
    }
    assert_eq!(
        shown(&mut layout, &state),
        shown(&mut Layout::default(), &state)
    );
}

fn shown(layout: &mut Layout, state: &State) -> String {
    for _ in 0..100 {
        layout.prepare(state, 80, 64);
        if layout.ready(state) {
            break;
        }
    }
    assert!(layout.ready(state));
    let mut rows: Vec<_> = layout.blocks.iter().flatten().map(Line::plain).collect();
    rows.extend(layout.pending(state).iter().map(Line::plain));
    rows.join("\n")
}

#[test]
fn reads_join_one_card_as_they_finish_and_the_keycap_follows_the_latest_turn() {
    let mut state = State::default();
    let mut layout = Layout::default();
    state.begin_tool_turn();
    read(&mut state, "a", "src/a.rs");
    finish_read(&mut state, "a");
    read(&mut state, "b", "src/b.rs");
    let text = shown(&mut layout, &state);
    assert!(text.contains("● Activity  2 calls · running"));
    assert!(text.contains("1 file read") && text.contains("Read  src/b.rs"));
    assert!(!text.contains("Read(src/a.rs)"));
    finish_read(&mut state, "b");
    let text = shown(&mut layout, &state);
    assert!(text.contains("● Activity  2 calls\n  ⎿ 2 files read"));
    assert!(!text.contains("● Read(src/a.rs)") && !text.contains("Ctrl+O"));
    state.close_tools();
    state.finish_tool_turn();
    let text = shown(&mut layout, &state);
    assert_eq!(text.matches("Ctrl+O  expand").count(), 1);
    assert_eq!(text.matches("● Activity  ").count(), 1);
    state.message(Kind::User, "Next request.");
    state.begin_tool_turn();
    assert!(!shown(&mut layout, &state).contains("Ctrl+O"));
}
