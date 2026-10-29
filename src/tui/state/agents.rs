//! Collaboration events in the transcript. Each is a finished tool row named
//! `collaboration` whose result is the event, so it joins Work chapters and
//! opens with Ctrl+O like any other call.

use super::{Item, State};
use crate::{
    json::Value,
    tui::cards::{self, Presentation},
};

fn label(event: &Value) -> String {
    match event.get("number").and_then(Value::as_usize) {
        Some(number) if number > 0 => format!("Agent {number}"),
        _ => "Agents".into(),
    }
}

/// Whether the runtime recorded the event outside any turn. Older events
/// carry no mark; only their restored unknown outcomes are known to be.
pub(in crate::tui) fn outside_turn(event: &Value) -> bool {
    match event.get("in_turn") {
        Some(in_turn) => in_turn == &Value::Bool(false),
        None => event.get("kind").and_then(Value::as_str) == Some("unknown"),
    }
}

impl State {
    /// Places an agent event from what it records. Inside a turn it joins
    /// the chapter of the agent call that produced it, otherwise the one it
    /// follows; an event that happened outside a turn is a lone row, with
    /// its own Ctrl+O turn when none is open. It never splits streaming
    /// prose: it lands before the stream, where a resumed conversation also
    /// places it.
    pub fn collaboration_event(&mut self, event: Value) {
        let streaming = matches!(self.items.last(), Some(Item::Streaming { .. }));
        let inside = self.tool_details.active;
        if !inside {
            self.close_tools();
            self.begin_tool_turn();
        }
        let joins = inside && !outside_turn(&event);
        let at = self.items.len() - usize::from(streaming);
        let turn_id = self.tool_details.turn_id;
        let previous = || {
            at.checked_sub(1)
                .and_then(|index| self.items[index].tool())
                .filter(|tool| {
                    tool.presentation.turn_id == turn_id && cards::chapter_groupable(tool)
                })
                .map(|tool| {
                    tool.arguments
                        .get("work")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_owned()
                })
        };
        let work = joins
            .then(|| {
                event
                    .get("work")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
                    .or_else(previous)
            })
            .flatten();
        let lone = work.is_none();
        // An open branch stays open so the chapter can keep growing.
        let open = inside && !streaming;
        if open && let Some(last) = self.open_branch() {
            *last = Some(false);
        }
        let item = Item::Tool {
            id: format!(
                "collaboration:{}",
                event.get("sequence").and_then(Value::as_usize).unwrap_or(0)
            ),
            name: "collaboration".into(),
            arguments: Value::object([
                ("label", Value::string(label(&event))),
                ("work", Value::string(work.unwrap_or_default())),
                ("lone", Value::Bool(lone)),
            ]),
            summary: String::new(),
            result: Some(event),
            change: None,
            last: if open { None } else { Some(true) },
            presentation: Presentation {
                turn_id,
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

    /// Places a saved event where it was shown live. Replay keeps a recorded
    /// turn open until the next one starts, so an event recorded outside a
    /// turn closes it and stays lone; one recorded in a turn after that
    /// reopens it for the continuation that followed.
    pub fn resumed_collaboration(&mut self, event: Value, turn_recorded: bool) {
        if outside_turn(&event) {
            self.finish_tool_turn();
        } else if turn_recorded && !self.tool_details.active {
            self.begin_tool_turn();
        }
        self.collaboration_event(event);
    }

    /// Older saved events carry no call label. Resumed conversations record
    /// an agent call after the events it caused; those events take the
    /// call's work label and join its chapter.
    pub fn adopt_collaboration(&mut self, indices: &[usize], work: &str) {
        for &index in indices {
            if let Some(Item::Tool {
                name, arguments, ..
            }) = self.items.get_mut(index)
                && name == "collaboration"
            {
                *arguments = Value::object([
                    (
                        "label",
                        arguments.get("label").cloned().unwrap_or(Value::Null),
                    ),
                    ("work", Value::string(work)),
                    ("lone", Value::Bool(false)),
                ]);
            }
        }
        for &index in indices {
            if let Some(Item::Tool { presentation, .. }) = self.items.get_mut(index) {
                presentation.chapter_start = None;
            }
            if index < self.items.len() {
                self.cache_chapter(index);
            }
        }
        self.changed();
        self.output_changed();
        self.update_chapters(indices);
    }
}
