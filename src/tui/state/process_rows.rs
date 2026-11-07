//! Process completions in the transcript. Each is a finished `process` row
//! whose result is the event, so it reads and opens like any other call.

use super::{Item, State};
use crate::{json::Value, tui::cards::Presentation};
use std::time::Duration;

impl State {
    /// Begins the turn a completion started. The rows that prompted it stay in
    /// the range Ctrl+O opens when the turn finishes.
    pub fn begin_process_turn(&mut self) {
        let start = self
            .items
            .iter()
            .rposition(|item| !matches!(item, Item::Tool { id, .. } if id.starts_with("process:")))
            .map_or(0, |last| last + 1);
        self.begin_tool_turn();
        if let Some(turn) = &mut self.tool_details.turn {
            turn.start = start;
        }
    }

    /// Places a completion at its position, as agent events are placed:
    /// inside a turn it joins the open chapter; outside one it is a lone row
    /// with its own Ctrl+O turn. Earlier rows and streaming prose stay as they are.
    pub fn process_completion(&mut self, result: Value) {
        let streaming = matches!(self.items.last(), Some(Item::Streaming { .. }));
        let inside = self.tool_details.active;
        if !inside {
            self.close_tools();
            self.begin_tool_turn();
        }
        let at = self.items.len() - usize::from(streaming);
        // An open branch stays open so the chapter can keep growing.
        let open = inside && !streaming;
        if open && let Some(last) = self.open_branch() {
            *last = Some(false);
        }
        let text = |key| {
            result
                .get(key)
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned()
        };
        let id = text("process_id");
        let elapsed = result
            .get("elapsed_ms")
            .and_then(Value::as_usize)
            .map(|milliseconds| Duration::from_millis(milliseconds as u64));
        let item = Item::Tool {
            id: format!("process:{id}"),
            name: "process".into(),
            arguments: Value::object([
                ("command", Value::string(text("command"))),
                ("process_id", Value::string(id)),
            ]),
            summary: crate::events::tool_summary("process", &result),
            result: Some(result),
            change: None,
            last: if open { None } else { Some(true) },
            presentation: Presentation {
                elapsed,
                turn_id: self.tool_details.turn_id,
                ..Presentation::default()
            },
        };
        self.items.insert(at, item);
        self.changed();
        self.output_changed();
        self.cache_chapter(at);
        self.update_chapters(&[at]);
        if !inside {
            self.finish_tool_turn();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{events::Event, tui::state::Kind};

    fn finished(id: &str, code: usize) -> Value {
        Value::object([
            ("process_id", Value::string(id)),
            ("command", Value::string("cargo test --locked")),
            ("status", Value::string("exited")),
            ("exit_code", Value::number(code)),
            ("elapsed_ms", Value::number(134_000)),
            ("stdout", Value::string("")),
            ("stderr", Value::string("")),
        ])
    }

    fn rows(state: &State) -> String {
        state
            .rows()
            .iter()
            .flatten()
            .map(|row| row.plain())
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn state() -> State {
        State {
            width: 80,
            height: 24,
            ..State::default()
        }
    }

    #[test]
    fn outside_a_turn_it_is_a_lone_row_named_after_its_command() {
        let mut state = state();
        state.message(Kind::User, "Build it.");
        state.process_completion(finished("process-1", 1));
        assert!(!state.tool_details.active);
        assert!(state.tool_details_available(), "Ctrl+O opens this row");
        let shown = rows(&state);
        assert!(shown.contains("PROCESS"), "{shown}");
        assert!(shown.contains("cargo test --locked"), "{shown}");
        assert!(shown.contains("Exit 1"), "{shown}");
    }

    #[test]
    fn a_process_killed_by_a_signal_is_not_a_success() {
        let mut state = state();
        let mut killed = finished("process-1", 0);
        if let Value::Object(fields) = &mut killed {
            fields.insert("exit_code".into(), Value::Null);
        }
        state.process_completion(killed);
        let shown = rows(&state);
        assert!(shown.contains("Exit unknown"), "{shown}");
    }

    #[test]
    fn a_success_is_quiet_with_its_time() {
        let mut state = state();
        state.process_completion(finished("process-1", 0));
        let shown = rows(&state);
        assert!(shown.contains("Done in 2m 14s"), "{shown}");
    }

    #[test]
    fn inside_a_turn_it_joins_the_chapter_and_never_splits_streaming_prose() {
        let mut state = state();
        state.begin_tool_turn();
        state.event(Event::ToolStarted {
            id: "one".into(),
            name: "bash".into(),
            arguments: Value::object([("command", Value::string("true"))]),
        });
        state.event(Event::ToolFinished {
            id: "one".into(),
            name: "bash".into(),
            summary: "exit 0".into(),
            result: Value::object([("exit_code", Value::number(0))]),
        });
        state.event(Event::Streaming {
            text: "Working on it".into(),
        });
        state.process_completion(finished("process-1", 0));
        assert!(state.tool_details.active);
        assert!(matches!(state.items.last(), Some(Item::Streaming { .. })));
        let shown = rows(&state);
        assert!(shown.contains("WORK"), "{shown}");
        assert!(shown.contains("cargo test --locked"), "{shown}");
    }
}
