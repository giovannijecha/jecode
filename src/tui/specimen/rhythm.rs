//! Identical synthetic events for before/after review through the Rust renderer.
use crate::{
    events::{Event, tool_summary},
    json::Value,
    tui::{
        state::{Kind, State},
        theme::STRONG,
    },
};
use std::time::Duration;

fn call(state: &mut State, id: &str, name: &str, arguments: Value, result: Value) {
    state.event(Event::ToolStarted {
        id: id.into(),
        name: name.into(),
        arguments,
    });
    state.event(Event::ToolFinished {
        id: id.into(),
        name: name.into(),
        summary: tool_summary(name, &result),
        result,
    });
}

fn scenario(columns: usize) -> State {
    let mut state = State {
        width: columns,
        ..State::default()
    };
    state.message(Kind::Assistant, "I will inspect the tool renderer and compare the narrow layout. The command and its result should read as work.\n\nThen I will check the browser failure and keep the full output available.");
    call(
        &mut state,
        "long",
        "bash",
        Value::object([
            (
                "command",
                Value::string(
                    "cargo test --locked --offline --all-targets -- tui::cards::rhythm_tests --nocapture && echo 'renderer checked at both terminal widths'",
                ),
            ),
            ("description", Value::string("Check the tool renderer")),
            ("work", Value::string("Verify rendering")),
        ]),
        Value::object([
            ("exit_code", Value::number(0)),
            (
                "stdout",
                Value::string(
                    "running checks\nchecking wrapping\nchecking colours\nAll checks passed\n",
                ),
            ),
            (
                "checkpoint",
                Value::object([
                    ("status", Value::string("none")),
                    ("reason", Value::string("No repository changes")),
                ]),
            ),
        ]),
    );
    state.message(
        Kind::Assistant,
        "The checks passed. I will inspect the browser state next.",
    );
    state.begin_tool_turn();
    call(
        &mut state,
        "read",
        "read",
        Value::object([
            ("path", Value::string("src/tui/cards.rs")),
            ("work", Value::string("Inspect browser state")),
        ]),
        Value::object([("lines_returned", Value::number(20))]),
    );
    call(
        &mut state,
        "browser",
        "browser",
        Value::object([
            ("action", Value::string("evaluate")),
            (
                "expression",
                Value::string("document.querySelector('[data-result]').textContent"),
            ),
            ("description", Value::string("Read the result label")),
            ("work", Value::string("Inspect browser state")),
        ]),
        Value::object([(
            "error",
            Value::string(
                "Browser JavaScript error: {\"text\":\"Uncaught\",\"exception\":{\"description\":\"TypeError: Cannot read properties of null (reading 'textContent')\\n    at <anonymous>:1:40\"}}",
            ),
        )]),
    );
    state.close_tools();
    state.finish_tool_turn();
    state.message(
        Kind::Assistant,
        "The selector needs inspection while the preview remains in the background.",
    );
    call(
        &mut state,
        "background",
        "process",
        Value::object([
            ("action", Value::string("poll")),
            ("process_id", Value::string("p1")),
            ("description", Value::string("Serve the local preview")),
        ]),
        Value::object([
            ("process_id", Value::string("p1")),
            ("status", Value::string("running")),
            ("command", Value::string("node .design/SERVE.cjs")),
            (
                "stdout",
                Value::string("Listening on http://127.0.0.1:6197\n"),
            ),
        ]),
    );
    state.message(Kind::Assistant, "I will run one more focused check.");
    state.event(Event::ToolStarted {
        id: "running".into(),
        name: "bash".into(),
        arguments: Value::object([
            (
                "command",
                Value::string("cargo clippy --locked --offline --all-targets -- -D warnings"),
            ),
            ("description", Value::string("Check compiler warnings")),
        ]),
    });
    for item in &mut state.items {
        if let super::super::state::Item::Tool { presentation, .. } = item {
            presentation.started = None;
            presentation.elapsed = Some(Duration::from_secs(3));
        }
    }
    state
}

#[test]
#[ignore = "prints native rhythm comparison at 80 and 40 cells"]
fn rhythm() {
    for columns in [80, 40] {
        let mut state = scenario(columns);
        for expanded in [false, true] {
            // Show each turn as it looks after its own Ctrl+O expansion.
            if expanded {
                for item in &mut state.items {
                    if let super::super::state::Item::Tool { presentation, .. } = item {
                        presentation.expanded = true;
                    }
                }
            }
            println!(
                "RHYTHM {columns} {}",
                if expanded { "expanded" } else { "compact" }
            );
            println!(
                "{}",
                super::Line::new("Jecode tool rhythm", STRONG.bold()).paint(columns)
            );
            for block in state.rows() {
                for line in block {
                    assert!(crate::tui::text::cells(&line.plain()) <= columns);
                    println!("{}", line.paint(columns));
                }
            }
        }
    }
}
