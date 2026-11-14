//! Synthetic input for the shared native renderer and local design comparisons.
use std::{fs, time::Duration};

use crate::{
    events::{Event, tool_summary},
    file_change::Change,
    json::{self, Value},
    tui::{
        line::Line,
        motion,
        state::{Item, Kind, State},
        text, theme,
        transcript::Layout,
    },
};

fn string<'a>(value: &'a Value, key: &str) -> &'a str {
    value.get(key).and_then(Value::as_str).unwrap_or("")
}

fn number(value: &Value, key: &str) -> usize {
    value.get(key).and_then(Value::as_usize).unwrap_or(0)
}

fn flag(value: &Value, key: &str) -> bool {
    value.get(key) == Some(&Value::Bool(true))
}

pub(in crate::tui) fn cases() -> Vec<Value> {
    let path = std::env::var_os("JECODE_WORK_INPUT")
        .expect("Set JECODE_WORK_INPUT to the synthetic work comparison JSON file");
    let data = json::parse(&fs::read_to_string(path).expect("Could not read work fixtures"))
        .expect("Invalid work fixture JSON");
    data.as_array()
        .expect("Work fixtures must be an array")
        .to_vec()
}

fn add_tool(state: &mut State, item: &Value) {
    let id = string(item, "id");
    let name = string(item, "name");
    state.event(Event::ToolStarted {
        id: id.into(),
        name: name.into(),
        arguments: item
            .get("arguments")
            .cloned()
            .unwrap_or_else(|| Value::object([])),
    });
    if let Some(after) = item.get("after").and_then(Value::as_str)
        && let Some(change) = Change::between(item.get("before").and_then(Value::as_str), after)
    {
        state.event(Event::ToolChanged {
            id: id.into(),
            change,
        });
    }
    if let Some(result) = item.get("result").filter(|value| **value != Value::Null) {
        state.event(Event::ToolFinished {
            id: id.into(),
            name: name.into(),
            summary: tool_summary(name, result),
            result: result.clone(),
        });
    }
}

pub(in crate::tui) fn state(case: &Value) -> State {
    let mut state = State {
        width: number(case, "cols"),
        height: 40,
        ..State::default()
    };
    state.begin_tool_turn();
    let items = case
        .get("items")
        .and_then(Value::as_array)
        .expect("Missing work items");
    for item in items {
        if string(item, "kind") == "message" {
            state.message(
                if string(item, "role") == "user" {
                    Kind::User
                } else {
                    Kind::Assistant
                },
                string(item, "text"),
            );
        } else {
            add_tool(&mut state, item);
        }
    }
    state.close_tools();
    if !flag(case, "active") {
        state.finish_tool_turn();
    }
    for item in items
        .iter()
        .filter(|item| string(item, "kind") != "message")
    {
        if let Some(Item::Tool { presentation, .. }) = state.items.iter_mut().find(
            |candidate| matches!(candidate, Item::Tool { id, .. } if id == string(item, "id")),
        ) {
            presentation.started = None;
            presentation.elapsed = Some(Duration::from_millis(number(item, "elapsed_ms") as u64));
        }
    }
    if flag(case, "expanded") && !flag(case, "active") {
        state
            .toggle_tools()
            .expect("The comparison turn must contain tools");
    }
    state
}

pub(in crate::tui) fn rows(case: &Value) -> Vec<Line> {
    theme::set_light(flag(case, "light"));
    motion::set_frame(8);
    let state = state(case);
    let mut layout = Layout::default();
    for _ in 0..4096 {
        layout.prepare(&state, state.width, 256);
        layout.prepare_pending(&state, 8192);
        if layout.ready(&state) && layout.pending_ready(&state) {
            let rows: Vec<_> = layout
                .blocks
                .iter()
                .skip(1)
                .flatten()
                .chain(layout.pending_blocks().iter().flatten())
                .cloned()
                .collect();
            assert!(
                rows.iter()
                    .all(|line| text::cells(&line.plain()) <= state.width)
            );
            return rows;
        }
    }
    panic!("Work fixture layout did not settle");
}

fn encoded(rows: &[Line]) -> Value {
    Value::Array(
        rows.iter()
            .map(|line| {
                Value::object([
                    (
                        "background",
                        line.background
                            .map_or(Value::Null, |surface| Value::string(surface.to_string())),
                    ),
                    (
                        "spans",
                        Value::Array(
                            line.spans
                                .iter()
                                .map(|span| {
                                    Value::Array(vec![
                                        Value::string(&span.text),
                                        Value::string(span.style.to_string()),
                                        span.background.map_or(Value::Null, |surface| {
                                            Value::string(surface.to_string())
                                        }),
                                    ])
                                })
                                .collect(),
                        ),
                    ),
                ])
            })
            .collect(),
    )
}

#[test]
#[ignore = "compares synthetic input using the real Rust layout; requires JECODE_WORK_INPUT"]
fn catalog_work() {
    let results: Vec<_> = cases()
        .iter()
        .map(|case| {
            let output = rows(case);
            Value::object([
                ("id", Value::string(string(case, "id"))),
                ("rows", encoded(&output)),
            ])
        })
        .collect();
    if let Some(path) = std::env::var_os("JECODE_WORK_OUTPUT") {
        fs::write(path, Value::Array(results).encode())
            .expect("Could not write native work comparison");
    } else {
        for result in &results {
            println!("@@WORK {}", result.encode());
        }
    }
    theme::set_light(false);
    motion::set_frame(0);
}
