use super::{Terminal, multi_edit_cards::shown};
use crate::{events::Event, file_change::Change, json::Value, tui::state::State};

/// A steps call through the native console and the full frame renderer: each
/// step is an ordinary card in the order it ran, and a refused call names its
/// tools (B20).
pub(super) fn console(terminal: &Terminal) {
    let (_, height) = terminal.size;
    for columns in [44, 80] {
        let mut state = State {
            width: columns,
            height,
            ..State::default()
        };
        state.begin_tool_turn();
        state.event(Event::ToolStarted {
            id: "chain_s1".into(),
            name: "edit".into(),
            arguments: Value::object([("path", Value::string("src/limit.rs"))]),
        });
        state.event(Event::ToolChanged {
            id: "chain_s1".into(),
            change: Change::between(Some("let limit = 10;\n"), "let limit = LIMIT;\n").unwrap(),
        });
        state.event(Event::ToolFinished {
            id: "chain_s1".into(),
            name: "edit".into(),
            summary: "edited file".into(),
            result: Value::object([("bytes_written", Value::number(19))]),
        });
        state.event(Event::ToolStarted {
            id: "chain_s2".into(),
            name: "bash".into(),
            arguments: Value::object([("command", Value::string("cargo test"))]),
        });
        let tests = Value::object([
            ("exit_code", Value::number(0)),
            ("stdout", Value::string("test result: ok\n")),
        ]);
        state.event(Event::ToolFinished {
            id: "chain_s2".into(),
            name: "bash".into(),
            summary: crate::events::tool_summary("bash", &tests),
            result: tests,
        });
        shown(
            terminal,
            state.rows().into_iter().flatten().collect(),
            columns,
            &[" UPDATE  src/limit.rs", "cargo test", "test result: ok"],
        );

        let mut state = State {
            width: columns,
            height,
            ..State::default()
        };
        state.begin_tool_turn();
        let step = |tool: &str| Value::object([("tool", Value::string(tool))]);
        state.event(Event::ToolStarted {
            id: "chain".into(),
            name: "steps".into(),
            arguments: Value::object([(
                "steps",
                Value::Array(vec![step("edit"), step("ask_user")]),
            )]),
        });
        let refused = Value::object([("error", Value::string("ask_user cannot run as a step"))]);
        state.event(Event::ToolFinished {
            id: "chain".into(),
            name: "steps".into(),
            summary: crate::events::tool_summary("steps", &refused),
            result: refused,
        });
        shown(
            terminal,
            state.rows().into_iter().flatten().collect(),
            columns,
            &[" STEPS  edit, ask_user", "ask_user cannot run as a step"],
        );
    }
}
