use super::*;
use crate::events::Event;

fn complete(layout: &mut Layout, state: &State, width: usize) {
    for _ in 0..1000 {
        layout.prepare(state, width, 64);
        if layout.ready(state) {
            return;
        }
    }
    panic!("Markdown reconstruction did not finish");
}

#[test]
fn growing_streamed_tables_rebuild_from_source_and_settle_once() {
    let mut state = State::default();
    state.message(Kind::User, "Compare the options.");
    let mut layout = Layout::default();
    complete(&mut layout, &state, 79);
    let settled = layout.blocks.clone();
    let final_source =
        "| Name | State |\n| --- | --- |\n| **first** | longer final value |\n| second | ready |";
    for source in [
        "| Name | State |",
        "| Name | State |\n| --- | unfinished |",
        "| Name | State |\n| --- | --- |\n| first | short |",
        final_source,
    ] {
        state.event(Event::Streaming {
            text: source.into(),
        });
        complete(&mut layout, &state, 79);
        assert_eq!(layout.blocks, settled, "streamed rows stay provisional");
        assert_eq!(
            layout.pending(&state),
            state.rows().last().cloned().unwrap()
        );
    }
    let streamed = layout.pending(&state);
    assert!(
        streamed
            .iter()
            .any(|row| row.plain().contains("longer final value"))
    );
    state.event(Event::Message {
        text: final_source.into(),
    });
    complete(&mut layout, &state, 79);
    assert!(layout.pending(&state).is_empty());
    assert_eq!(layout.blocks.last().unwrap(), &streamed);
    let wide = layout.blocks.clone();
    complete(&mut layout, &state, 10);
    assert!(
        layout
            .blocks
            .last()
            .unwrap()
            .iter()
            .any(|row| row.plain().ends_with("Name:"))
    );
    complete(&mut layout, &state, 79);
    assert_eq!(layout.blocks, wide);
    assert!(matches!(state.items.last(), Some(Item::Text { text, .. }) if text == final_source));
}

#[test]
fn table_measurement_yields_and_a_resize_discards_the_partial_width_layout() {
    let source = format!(
        "| Name | State |\n|---|---|\n{}",
        (0..600)
            .map(|i| format!("| item{i:03} | ready |\n"))
            .collect::<String>()
    );
    let mut state = State::default();
    state.message(Kind::Assistant, &source);
    let mut layout = Layout::default();
    layout.prepare(&state, 40, 64);
    assert_eq!(layout.blocks.len(), 1);
    assert!(!layout.ready(&state));
    assert!(layout.partial.as_ref().unwrap().lines.is_empty());
    complete(&mut layout, &state, 10);
    let narrow = layout.blocks.last().unwrap();
    assert_eq!(
        narrow
            .iter()
            .filter(|row| row.plain().ends_with("Name:"))
            .count(),
        600
    );
    complete(&mut layout, &state, 40);
    let wide = layout.blocks.last().unwrap();
    assert_eq!(wide, &prose::render(&source, 40));
    assert_eq!(wide.len(), 602);
    assert_eq!(
        wide.iter()
            .filter(|row| row.plain().contains("item599"))
            .count(),
        1
    );
}

#[test]
fn a_single_tall_table_cell_yields_while_emitting_wrapped_rows() {
    let source = format!("| A | B |\n|---|---|\n| {} | z |", "value ".repeat(10_000));
    let mut state = State::default();
    state.message(Kind::Assistant, &source);
    let mut layout = Layout::default();
    layout.prepare(&state, 16, 64);
    assert!(!layout.ready(&state));
    let first = layout.partial.as_ref().unwrap().lines.len();
    assert!(
        first <= 256 + 32,
        "only one bounded table batch may cross the allowance"
    );
    layout.prepare(&state, 16, 64);
    assert!(!layout.ready(&state));
    let second = layout.partial.as_ref().unwrap().lines.len();
    assert!(second > first && second - first <= 256 + 32);
    complete(&mut layout, &state, 16);
    let body = layout.blocks.last().unwrap();
    assert_eq!(
        body.iter()
            .map(Line::plain)
            .collect::<String>()
            .matches("value")
            .count(),
        10_000
    );
}

#[test]
fn live_stream_renders_a_bounded_prefix_and_discards_replaced_or_removed_text() {
    let mut state = State::default();
    let first = (0..600)
        .map(|index| format!("Line {index}\n\n"))
        .collect::<String>();
    state.event(Event::Streaming { text: first });
    let mut layout = Layout::default();
    layout.prepare(&state, 40, 64);
    layout.prepare_pending(&state, 32);
    assert!(!layout.pending_ready(&state));
    // A pass emits a bounded number of complete source lines.
    let prefix: Vec<_> = layout.pending_blocks()[0].iter().map(Line::plain).collect();
    assert_eq!(&prefix[..3], ["● Line 0", "  ", "  Line 1"]);
    assert!(prefix.len() <= 32);
    assert!(!prefix.iter().any(|line| line == "Line 599"));

    let replacement = "# Replacement\nFinal response.";
    state.event(Event::Streaming {
        text: replacement.into(),
    });
    layout.prepare_pending(&state, 32);
    assert!(layout.pending_ready(&state));
    assert_eq!(layout.pending_blocks()[0], prose::render(replacement, 40));

    state.event(Event::RequestDiscarded);
    layout.prepare(&state, 40, 64);
    layout.prepare_pending(&state, 32);
    assert!(layout.pending_ready(&state));
    assert!(layout.pending_blocks().is_empty());
}

#[test]
fn empty_stream_snapshot_emits_no_phantom_row_before_visible_text() {
    let mut state = State::default();
    state.message(Kind::User, "Prompt");
    state.event(Event::Streaming {
        text: " \n ".into(),
    });
    assert_eq!(state.items.len(), 1);

    // A retained empty snapshot can also arrive from an older session.
    state.items.push(Item::Streaming {
        text: " \n ".into(),
    });
    let mut layout = Layout::default();
    complete(&mut layout, &state, 40);
    layout.prepare_pending(&state, 32);
    assert!(layout.pending_ready(&state));
    assert_eq!(layout.pending_blocks(), &[Vec::<Line>::new()]);

    state.event(Event::Streaming {
        text: "Answer".into(),
    });
    layout.prepare_pending(&state, 32);
    assert!(layout.pending_ready(&state));
    assert_eq!(
        layout.pending_blocks()[0]
            .iter()
            .map(Line::plain)
            .collect::<Vec<_>>(),
        ["", "● Answer"]
    );
}

#[test]
fn repeated_appends_keep_exactly_one_separator_and_match_finalized_rows() {
    let mut state = State {
        width: 40,
        ..State::default()
    };
    state.message(Kind::User, "Explain the result.");
    let mut layout = Layout::default();
    for source in [
        "First line",
        "First line\nSecond",
        "First line\nSecond line\nThird",
        "First line\nSecond line\nThird line",
    ] {
        state.event(Event::Streaming {
            text: source.into(),
        });
        complete(&mut layout, &state, 40);
        for _ in 0..100 {
            layout.prepare_pending(&state, 2);
            if layout.pending_ready(&state) {
                break;
            }
        }
        assert!(layout.pending_ready(&state));
        assert_eq!(
            layout.pending_blocks()[0],
            state.rows().last().unwrap().clone()
        );
        assert_eq!(layout.pending_blocks()[0][0].plain(), "");
        assert_eq!(
            layout.pending_blocks()[0]
                .iter()
                .filter(|line| line.plain().is_empty())
                .count(),
            1
        );
    }

    let visible = layout.pending_blocks()[0].clone();
    state.event(Event::Message {
        text: "First line\nSecond line\nThird line".into(),
    });
    complete(&mut layout, &state, 40);
    layout.prepare_pending(&state, 2);
    assert!(layout.pending_blocks().is_empty());
    assert_eq!(layout.blocks.last().unwrap(), &visible);

    state.width = 20;
    complete(&mut layout, &state, 20);
    assert_eq!(layout.blocks.last().unwrap(), state.rows().last().unwrap());
}

#[test]
fn replaced_stream_and_width_reset_retain_the_predecessor_separator() {
    let mut state = State {
        width: 40,
        ..State::default()
    };
    state.message(Kind::User, "Request");
    let mut layout = Layout::default();
    for source in [
        "Original response\nwith a second line",
        "Replacement response",
    ] {
        state.event(Event::Streaming {
            text: source.into(),
        });
        complete(&mut layout, &state, 40);
        layout.prepare_pending(&state, 256);
        assert!(layout.pending_ready(&state));
        assert_eq!(
            layout.pending_blocks()[0],
            state.rows().last().unwrap().clone()
        );
    }
    state.width = 20;
    complete(&mut layout, &state, 20);
    layout.prepare_pending(&state, 256);
    assert!(layout.pending_ready(&state));
    assert_eq!(
        layout.pending_blocks()[0],
        state.rows().last().unwrap().clone()
    );
    assert_eq!(layout.pending_blocks()[0][0].plain(), "");
}
