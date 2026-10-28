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

impl State {
    /// Places an agent event. `call` is the work label of the running agent
    /// call that caused it. Inside a turn an event joins the chapter it
    /// follows; outside a turn it is a lone row with its own Ctrl+O turn. It
    /// never splits streaming prose: it lands before the stream, where a
    /// resumed conversation also places it.
    pub fn collaboration_event(&mut self, event: Value, call: Option<&str>) {
        let streaming = matches!(self.items.last(), Some(Item::Streaming { .. }));
        let inside = self.tool_details.active;
        if !inside {
            self.close_tools();
            self.begin_tool_turn();
        }
        let at = self.items.len() - usize::from(streaming);
        let turn_id = self.tool_details.turn_id;
        let previous = at
            .checked_sub(1)
            .and_then(|index| self.items[index].tool())
            .filter(|tool| {
                inside && tool.presentation.turn_id == turn_id && cards::chapter_groupable(tool)
            })
            .map(|tool| {
                tool.arguments
                    .get("work")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_owned()
            });
        let work = call.map(str::to_owned).or(previous);
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

    /// Resumed conversations record an agent call after the events it caused;
    /// those events take the call's work label and join its chapter.
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
