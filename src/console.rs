use crate::events::description;
use crate::events::{Event, EventSink};
use std::collections::HashMap;
use std::io::Write;

struct ActiveCall {
    target: String,
    distinguish: bool,
}

pub struct Console<'a, O, E> {
    output: &'a mut O,
    status: &'a mut E,
    waiting_shown: bool,
    active: HashMap<String, ActiveCall>,
}

impl<'a, O: Write, E: Write> Console<'a, O, E> {
    pub fn new(output: &'a mut O, status: &'a mut E) -> Self {
        Self {
            output,
            status,
            waiting_shown: false,
            active: HashMap::new(),
        }
    }

    /// Questions are asked through the chat's own input flow.
    pub fn output(&mut self) -> &mut O {
        self.output
    }
}

impl<O: Write, E: Write> EventSink for Console<'_, O, E> {
    fn emit(&mut self, event: Event) -> Result<(), String> {
        let text = match event {
            Event::Collaboration { event } => collaboration_line(&event),
            Event::Message { text } => {
                return writeln!(self.output, "{text}")
                    .and_then(|_| self.output.flush())
                    .map_err(|error| format!("Could not display agent output: {error}"));
            }
            Event::Waiting { model } => {
                if self.waiting_shown {
                    return Ok(());
                }
                self.waiting_shown = true;
                format!("[waiting for model: {model}]")
            }
            Event::ToolStarted {
                id,
                name,
                arguments,
            } => {
                let target = clean(&description(&name, &arguments));
                let overlap = !self.active.is_empty();
                if overlap {
                    for call in self.active.values_mut() {
                        call.distinguish = true;
                    }
                }
                let explicit = arguments.get("parallel") == Some(&crate::json::Value::Bool(true));
                self.active.insert(
                    id,
                    ActiveCall {
                        target: target.clone(),
                        distinguish: overlap || explicit,
                    },
                );
                format!("[tool: {name}] {target}")
            }
            Event::ToolFinished {
                id, name, summary, ..
            } => {
                let suffix = self
                    .active
                    .remove(&id)
                    .filter(|call| call.distinguish && !call.target.is_empty())
                    .map_or_else(String::new, |call| format!(" {}", call.target));
                format!("[{name}: {summary}]{suffix}")
            }
            Event::Preparing => {
                self.active.clear();
                return Ok(());
            }
            Event::ToolNote { text, .. } => format!("[{text}]"),
            Event::Recovering {
                attempt,
                delay,
                error,
                ..
            } => format!(
                "[reconnecting: attempt {attempt}, waiting {:.1}s] {error}",
                delay.as_secs_f64()
            ),
            Event::Maintenance { text } | Event::ContextCompacted { text } => {
                format!("[context] {text}")
            }
            Event::Reasoning
            | Event::Working
            | Event::Streaming { .. }
            | Event::ToolChanged { .. }
            | Event::RecoveryFinished
            | Event::BackendFailure { .. }
            | Event::RequestDiscarded
            | Event::Accepted { .. }
            | Event::InputHeld
            | Event::UserQuestion { .. }
            | Event::Context(_)
            | Event::Generated { .. } => return Ok(()),
        };
        writeln!(self.status, "{text}")
            .and_then(|_| self.status.flush())
            .map_err(|error| format!("Could not display agent output: {error}"))
    }
}

fn clean(value: &str) -> String {
    value
        .chars()
        .map(|ch| if ch.is_control() { ' ' } else { ch })
        .collect()
}

pub(crate) fn collaboration_line(event: &crate::json::Value) -> String {
    let text = |key| {
        event
            .get(key)
            .and_then(crate::json::Value::as_str)
            .unwrap_or("")
    };
    let label = text("id")
        .strip_prefix('a')
        .map_or_else(|| "AGENTS".to_owned(), |number| format!("AGENT {number}"));
    clean(&format!("[{label}: {}] {}", text("title"), text("summary")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::json::Value;

    #[test]
    fn collaborator_calls_show_actions_and_numbered_identity() {
        let (mut output, mut status) = (Vec::new(), Vec::new());
        let mut console = Console::new(&mut output, &mut status);
        console
            .emit(Event::ToolStarted {
                id: "call".into(),
                name: "agent".into(),
                arguments: Value::object([
                    ("action", Value::string("inspect")),
                    ("id", Value::string("a2")),
                ]),
            })
            .unwrap();
        assert_eq!(
            String::from_utf8(status).unwrap(),
            "[tool: agent] inspect AGENT 2\n"
        );
    }

    #[test]
    fn overlapping_finishes_name_their_targets_in_completion_order() {
        let (mut output, mut status) = (Vec::new(), Vec::new());
        let mut console = Console::new(&mut output, &mut status);
        let start = |id: &str, path: &str| Event::ToolStarted {
            id: id.into(),
            name: "read".into(),
            arguments: Value::object([
                ("path", Value::string(path)),
                ("description", Value::string(path)),
            ]),
        };
        let finish = |id: &str| Event::ToolFinished {
            id: id.into(),
            name: "read".into(),
            summary: "9 lines".into(),
            result: Value::object([]),
        };
        console.emit(start("a", "src/a.rs")).unwrap();
        console.emit(start("b", "src/b.rs")).unwrap();
        console.emit(finish("b")).unwrap();
        console.emit(finish("a")).unwrap();
        console.emit(start("c", "src/c.rs")).unwrap();
        console.emit(finish("c")).unwrap();
        drop(console);
        assert_eq!(
            String::from_utf8(status).unwrap(),
            "[tool: read] src/a.rs\n[tool: read] src/b.rs\n[read: 9 lines] src/b.rs\n[read: 9 lines] src/a.rs\n[tool: read] src/c.rs\n[read: 9 lines]\n"
        );
    }

    #[test]
    fn explicit_parallel_and_new_turn_reset_keep_receipts_unambiguous() {
        let (mut output, mut status) = (Vec::new(), Vec::new());
        let mut console = Console::new(&mut output, &mut status);
        console
            .emit(Event::ToolStarted {
                id: "first".into(),
                name: "read".into(),
                arguments: Value::object([
                    ("path", Value::string("src/first.rs")),
                    ("description", Value::string("First\nsource")),
                    ("parallel", Value::Bool(true)),
                ]),
            })
            .unwrap();
        console
            .emit(Event::ToolFinished {
                id: "first".into(),
                name: "read".into(),
                summary: "done".into(),
                result: Value::object([]),
            })
            .unwrap();
        console
            .emit(Event::ToolStarted {
                id: "unclosed".into(),
                name: "read".into(),
                arguments: Value::object([("path", Value::string("old.rs"))]),
            })
            .unwrap();
        console.emit(Event::Preparing).unwrap();
        console
            .emit(Event::ToolStarted {
                id: "next".into(),
                name: "read".into(),
                arguments: Value::object([
                    ("path", Value::string("new.rs")),
                    ("description", Value::string("New source")),
                ]),
            })
            .unwrap();
        console
            .emit(Event::ToolFinished {
                id: "next".into(),
                name: "read".into(),
                summary: "done".into(),
                result: Value::object([]),
            })
            .unwrap();
        drop(console);
        assert_eq!(
            String::from_utf8(status).unwrap(),
            "[tool: read] First source\n[read: done] First source\n[tool: read] old.rs (from line 1, up to 200 lines)\n[tool: read] New source\n[read: done]\n"
        );
    }
}
