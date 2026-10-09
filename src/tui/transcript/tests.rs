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
            .any(|line| line.plain() == "Live response.")
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
            .filter(|line| line.plain() == "Live response.")
            .count(),
        1
    );
}

#[test]
fn a_finished_tool_stays_live_until_the_tree_branch_is_settled() {
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
    assert!(layout.pending(&state)[0].plain().starts_with("├─"));
    state.close_tools();
    layout.prepare(&state, 39, 64);
    assert_eq!(layout.blocks.len(), 2);
    assert!(layout.blocks[1][0].plain().starts_with("└─"));
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
            .filter(|line| line.plain() == "comment line")
            .count(),
        600
    );
    assert_eq!(body[0].plain(), "rust");
    assert!(body[500].spans.iter().all(|span| span.style == MUTED));
    let code = body
        .iter()
        .find(|line| line.plain() == "let value = 1;")
        .unwrap();
    assert!(code.paint(39).contains(crate::tui::theme::KEYWORD));
    assert_eq!(body.last().unwrap().plain(), "Final paragraph.");
}
