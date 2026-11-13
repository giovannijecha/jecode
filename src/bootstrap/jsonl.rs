//! JSON Lines for `--json` runs: one object per line on stdout, in order.
//! Text and tool data pass through the conversation's redactor.

use crate::events::{Event, EventSink};
use crate::json::Value;
use crate::redact::Redactor;
use std::io::Write;

pub struct Lines<'a, O> {
    output: &'a mut O,
    redactor: Redactor,
}

impl<'a, O: Write> Lines<'a, O> {
    pub fn new(output: &'a mut O, redactor: Redactor) -> Self {
        Self { output, redactor }
    }

    pub fn write(&mut self, value: Value) -> Result<(), String> {
        writeln!(self.output, "{}", self.redactor.value(&value).encode())
            .and_then(|()| self.output.flush())
            .map_err(|error| format!("Could not write JSON output: {error}"))
    }
}

fn object(kind: &str, fields: Vec<(&str, Value)>) -> Value {
    let mut entries = vec![("type", Value::string(kind))];
    entries.extend(fields);
    Value::entries(entries)
}

pub fn event(event: Event) -> Option<Value> {
    Some(match event {
        Event::Message { text } => object("message", vec![("text", Value::string(text))]),
        Event::ToolStarted {
            id,
            name,
            arguments,
        } => object(
            "tool_started",
            vec![
                ("id", Value::string(id)),
                ("name", Value::string(name)),
                ("arguments", arguments),
            ],
        ),
        Event::ToolFinished {
            id, name, summary, ..
        } => object(
            "tool_finished",
            vec![
                ("id", Value::string(id)),
                ("name", Value::string(name)),
                ("summary", Value::string(summary)),
            ],
        ),
        Event::ToolNote { id, text } => object(
            "tool_note",
            vec![("id", Value::string(id)), ("text", Value::string(text))],
        ),
        Event::ProcessFinished { process } => object(
            "process_finished",
            vec![
                ("id", Value::string(&process.id)),
                ("command", Value::string(&process.command)),
                ("status", Value::string(process.label())),
            ],
        ),
        Event::Collaboration { event } => object("collaboration", vec![("event", event)]),
        Event::Recovering {
            attempt,
            delay,
            error,
            ..
        } => object(
            "retrying",
            vec![
                ("attempt", Value::number(attempt)),
                ("delay_ms", Value::number(delay.as_millis())),
                ("error", Value::string(error)),
            ],
        ),
        Event::BackendFailure { diagnostic, .. } => object(
            "backend_failure",
            vec![("error", Value::string(diagnostic))],
        ),
        Event::Maintenance { text } | Event::ContextCompacted { text } => {
            object("context", vec![("text", Value::string(text))])
        }
        Event::Generated { input, tokens } => object(
            "usage",
            vec![
                ("input_tokens", Value::number(input)),
                ("output_tokens", Value::number(tokens)),
            ],
        ),
        Event::UserQuestion { .. }
        | Event::PermissionRequest { .. }
        | Event::Preparing
        | Event::Waiting { .. }
        | Event::Reasoning
        | Event::Working
        | Event::RecoveryFinished
        | Event::RequestDiscarded
        | Event::Streaming { .. }
        | Event::Accepted { .. }
        | Event::Context(_)
        | Event::ToolChanged { .. } => return None,
    })
}

impl<O: Write> EventSink for Lines<'_, O> {
    fn emit(&mut self, event: Event) -> Result<(), String> {
        match self::event(event) {
            Some(value) => self.write(value),
            None => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn events_become_one_redacted_object_per_line() {
        let mut output = Vec::new();
        let redactor = Redactor::new("fixture-secret-value".into());
        let mut lines = Lines::new(&mut output, redactor);
        lines
            .emit(Event::Message {
                text: "uses fixture-secret-value".into(),
            })
            .unwrap();
        lines.emit(Event::Working).unwrap();
        lines
            .emit(Event::ToolFinished {
                id: "call".into(),
                name: "read".into(),
                summary: "9 lines".into(),
                result: Value::object([]),
            })
            .unwrap();
        let text = String::from_utf8(output).unwrap();
        let rows: Vec<_> = text.lines().collect();
        assert_eq!(rows.len(), 2, "{text}");
        let first = crate::json::parse(rows[0]).unwrap();
        assert_eq!(first.get("type").and_then(Value::as_str), Some("message"));
        assert!(!text.contains("fixture-secret-value"));
        let second = crate::json::parse(rows[1]).unwrap();
        assert_eq!(
            second.get("summary").and_then(Value::as_str),
            Some("9 lines")
        );
    }
}
