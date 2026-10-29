//! How engine events change the transcript, the status and the turn.

use super::{Item, Kind, State};
use crate::tui::{cards::Presentation, feedback::Feedback};
use crate::{backend::FailureReason, events::Event};

impl State {
    pub fn event(&mut self, event: Event) {
        if matches!(
            &event,
            Event::Streaming { .. }
                | Event::Message { .. }
                | Event::ToolStarted { .. }
                | Event::ToolChanged { .. }
                | Event::ToolNote { .. }
                | Event::ToolFinished { .. }
                | Event::Collaboration { .. }
                | Event::UserQuestion { .. }
        ) {
            self.changed();
        }
        match event {
            Event::Collaboration { event } => self.collaboration_event(event),
            Event::Recovering {
                attempt,
                delay,
                error,
                reason,
            } => {
                if matches!(self.items.last(), Some(Item::Streaming { .. })) {
                    self.items.pop();
                    self.changed();
                }
                self.status = "Waiting for connection…".into();
                // The rule counts down to the next attempt.
                let subject = if reason == Some(FailureReason::RateLimited) {
                    "Rate limited"
                } else {
                    "Reconnecting"
                };
                let recovery = Feedback::progress(
                    Kind::Warning,
                    format!("{subject} · attempt {attempt} · {error}"),
                );
                // Server delays can exceed the platform's Instant range. The
                // cancellable retry still honors them without a countdown.
                self.recovery = Some(match std::time::Instant::now().checked_add(delay) {
                    Some(until) => recovery.until(until),
                    None => recovery,
                });
            }
            Event::RecoveryFinished => {
                self.recovery = None;
            }
            Event::BackendFailure { reason, diagnostic } => {
                self.backend_failure = Some((reason, diagnostic));
            }
            Event::RequestDiscarded => {
                if matches!(self.items.last(), Some(Item::Streaming { .. })) {
                    self.items.pop();
                    self.changed();
                }
            }
            Event::Maintenance { text } => {
                self.notify(Feedback::progress(Kind::Notice, text));
            }
            Event::ContextCompacted { text } => {
                self.clear_progress();
                self.message(Kind::Notice, &text);
            }
            Event::Preparing => {
                self.backend_failure = None;
                self.status = "Preparing request…".into();
            }
            Event::Waiting { .. } => {
                self.status = "Waiting for model…".into();
            }
            Event::Reasoning => self.status = "Thinking…".into(),
            Event::Working => self.status = "Working…".into(),
            Event::InputHeld => {
                self.status = "Waiting for your edit…".into();
            }
            // The application shows accepted input with its prompt history.
            Event::Accepted { .. } => {}
            Event::UserQuestion { request } => {
                if self.question.as_ref().is_none_or(|current| current.id != request.id) {
                    self.ask_panel = Some(crate::tui::ask::Panel::new(&request, self.ask_draft.as_ref()));
                }
                self.question = Some(request);
            }
            Event::Context(status) => self.context = Some(status),
            Event::Generated { tokens } => {
                if let Some(activity) = &mut self.activity {
                    activity.generated += tokens;
                }
            }
            Event::Streaming { text } => {
                self.status = "Writing the response…".into();
                // Updates contain the entire response so far. Its leading whitespace
                // remains available when visible content arrives in a later update.
                if text.trim().is_empty() {
                    return;
                }
                if let Some(Item::Streaming { text: current }) = self.items.last_mut() {
                    if *current != text {
                        *current = text;
                        self.output_changed();
                    }
                } else {
                    self.close_tools();
                    self.items.push(Item::Streaming { text });
                    self.output_changed();
                }
            }
            Event::Message { text } => {
                if let Some(Item::Streaming { text: current }) = self.items.last() {
                    let changed = *current != text;
                    *self.items.last_mut().unwrap() = Item::Text {
                        kind: Kind::Assistant,
                        text,
                    };
                    if changed {
                        self.output_changed();
                    }
                } else {
                    self.message(Kind::Assistant, &text);
                }
            }
            Event::ToolStarted {
                id,
                name,
                arguments,
            } => {
                self.status = "Working…".into();
                if let Some(activity) = &mut self.activity {
                    activity.tools += 1;
                }
                // The plan lives in /todo; only the call creating one has a row.
                if name == "todo_list" && !self.todo_calls.creates(&arguments) {
                    return;
                }
                if let Some(last) = self.open_branch() {
                    *last = Some(false);
                }
                self.items.push(Item::Tool {
                    id,
                    name,
                    arguments,
                    summary: "running".into(),
                    result: None,
                    change: None,
                    last: None,
                    presentation: Presentation { turn_id: self.tool_details.turn_id, ..Presentation::live() },
                });
                let index = self.items.len() - 1;
                self.output_changed();
                self.cache_chapter(index);
                self.update_chapters(&[index]);
            }
            Event::ToolNote { id, text } => {
                if let Some(Item::Tool { presentation, .. }) = self.items.iter_mut().rev().find(
                    |item| matches!(item, Item::Tool { id: current, result: None, .. } if *current == id),
                ) {
                    presentation.note = Some(text);
                    self.output_changed();
                }
            }
            Event::ToolChanged { id, change } => {
                if let Some(Item::Tool { change: captured, .. }) = self.items.iter_mut().rev().find(|item| matches!(item, Item::Tool { id: current, result: None, .. } if *current == id)) {
                    *captured = Some(Box::new(change));
                    self.output_changed();
                }
            }
            Event::ToolFinished {
                id,
                name,
                summary,
                result,
            } => {
                if name == "todo_list" {
                    self.todo_calls.finished(&result);
                }
                if let Some(index) = self.items.iter().rposition(|item| {
                    matches!(item, Item::Tool { id: candidate, name: tool, result: None, .. }
                        if candidate == &id && tool == &name)
                }) && let Item::Tool {
                    summary: current,
                    result: captured,
                    presentation,
                    ..
                } = &mut self.items[index]
                {
                    *current = summary;
                    *captured = Some(result);
                    presentation.finish();
                    self.output_changed();
                    self.update_chapters(&[index]);
                }
            }
        }
    }
}
