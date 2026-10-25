use crate::events::description;
use crate::events::{Event, EventSink};
use std::io::Write;

pub struct Console<'a, O, E> {
    output: &'a mut O,
    status: &'a mut E,
    waiting_shown: bool,
}

impl<'a, O: Write, E: Write> Console<'a, O, E> {
    pub fn new(output: &'a mut O, status: &'a mut E) -> Self {
        Self {
            output,
            status,
            waiting_shown: false,
        }
    }
}

impl<O: Write, E: Write> EventSink for Console<'_, O, E> {
    fn emit(&mut self, event: Event) -> Result<(), String> {
        let text = match event {
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
                name, arguments, ..
            } => format!(
                "[tool: {name}] {}",
                description(&name, &arguments)
                    .chars()
                    .map(|ch| if ch.is_control() { ' ' } else { ch })
                    .collect::<String>()
            ),
            Event::ToolFinished { name, summary, .. } => format!("[{name}: {summary}]"),
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
            Event::Preparing
            | Event::Reasoning
            | Event::Working
            | Event::Streaming { .. }
            | Event::ToolChanged { .. }
            | Event::RecoveryFinished
            | Event::BackendFailure { .. }
            | Event::RequestDiscarded
            | Event::Accepted { .. }
            | Event::InputHeld
            | Event::Context(_) => return Ok(()),
        };
        writeln!(self.status, "{text}")
            .and_then(|_| self.status.flush())
            .map_err(|error| format!("Could not display agent output: {error}"))
    }
}
