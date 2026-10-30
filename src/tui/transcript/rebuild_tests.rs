use super::*;
use crate::{events::Event, json::Value};
use std::time::{Duration, Instant};

fn conversation(answer: &str) -> State {
    let mut state = State::default();
    state.begin_tool_turn();
    state.event(Event::ToolStarted {
        id: "read".into(),
        name: "read".into(),
        arguments: Value::object([("path", Value::string("fixture.rs"))]),
    });
    state.event(Event::ToolFinished {
        id: "read".into(),
        name: "read".into(),
        summary: "complete".into(),
        result: Value::object([("content", Value::string("fixture content"))]),
    });
    state.close_tools();
    state.message(Kind::Assistant, answer);
    state.finish_tool_turn();
    state
}

fn complete(layout: &mut Layout, state: &State) {
    for _ in 0..1000 {
        layout.prepare(state, 80, 64);
        if layout.ready(state) {
            return;
        }
    }
    panic!("fixture layout did not finish");
}

#[test]
fn tool_toggles_and_next_turn_rebuild_long_prose_in_bounded_passes() {
    let answer = format!(
        "```rust\n/*\n{}*/\nlet value = 1;\n```",
        "comment line\n".repeat(700)
    );
    let mut state = conversation(&answer);
    let mut layout = Layout::default();
    complete(&mut layout, &state);
    let answer = layout.blocks.last().unwrap().clone();
    for _ in 0..2 {
        state.toggle_tools().unwrap();
        layout.prepare(&state, 80, 64);
        assert!(
            !layout.ready(&state),
            "long prose must yield between passes"
        );
        assert_eq!(layout.blocks.last().unwrap(), &answer);
        complete(&mut layout, &state);
        let mut fresh = Layout::default();
        complete(&mut fresh, &state);
        assert_eq!(layout.blocks, fresh.blocks);
    }
    state.toggle_tools().unwrap();
    complete(&mut layout, &state);
    state.begin_tool_turn();
    layout.prepare(&state, 80, 64);
    assert!(!layout.ready(&state));
    complete(&mut layout, &state);
    let mut fresh = Layout::default();
    complete(&mut fresh, &state);
    assert_eq!(layout.blocks, fresh.blocks);
}

#[test]
fn a_tool_change_during_a_partial_rebuild_keeps_the_final_layout_exact() {
    let mut state = conversation(&"Paragraph with **emphasis**.\n\n".repeat(700));
    let mut layout = Layout::default();
    complete(&mut layout, &state);
    state.toggle_tools().unwrap();
    layout.prepare(&state, 80, 64);
    state.begin_tool_turn();
    complete(&mut layout, &state);
    let mut fresh = Layout::default();
    complete(&mut fresh, &state);
    assert_eq!(layout.blocks, fresh.blocks);
}

#[test]
#[ignore = "synthetic release layout benchmark; no network, configuration or terminal writes"]
fn long_answer_tool_toggle_measurement() {
    let answer = "Text with **emphasis**.\n\n".repeat(30_000);
    let mut state = conversation(&answer);
    let mut layout = Layout::default();
    complete(&mut layout, &state);
    let expected = layout.blocks.last().unwrap().clone();
    println!(
        "META,80 columns; {} bytes; 3 warmups; 5 samples; microseconds",
        answer.len()
    );
    for sample in 0..8 {
        state.toggle_tools().unwrap();
        let started = Instant::now();
        let mut longest = Duration::ZERO;
        let mut passes = 0;
        loop {
            let tick = Instant::now();
            layout.prepare(&state, 80, 64);
            longest = longest.max(tick.elapsed());
            passes += 1;
            if layout.ready(&state) {
                break;
            }
            assert!(passes < 1000);
        }
        let elapsed = started.elapsed();
        assert_eq!(layout.blocks.last().unwrap(), &expected);
        if sample >= 3 {
            println!(
                "RAW,{},{passes},{:.3},{:.3}",
                sample - 3,
                longest.as_secs_f64() * 1e6,
                elapsed.as_secs_f64() * 1e6,
            );
        }
    }
}
