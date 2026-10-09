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
            .any(|row| row.plain() == "Name:")
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
        narrow.iter().filter(|row| row.plain() == "Name:").count(),
        600
    );
    complete(&mut layout, &state, 40);
    let wide = layout.blocks.last().unwrap();
    assert_eq!(wide, &markdown::render(&source, 40));
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
