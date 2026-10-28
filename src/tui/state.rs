use super::{
    activity::Activity,
    cards::Presentation,
    drafts::{History, Queue},
    editor::Editor,
    feedback::Feedback,
    information::Information,
    suggestions::Suggestions,
};
use crate::json::Value;
use crate::{backend::FailureReason, events::Event};

pub(super) mod rows;

#[derive(Clone, Copy)]
pub enum Kind {
    User,
    Assistant,
    Notice,
    Warning,
    Error,
}
/// Header facts and the conversations offered until the first prompt.
#[derive(Default)]
pub struct Opening {
    /// Up to three other conversations in this folder, newest first.
    pub recent: Vec<Recent>,
}
pub struct Recent {
    pub title: String,
    pub age: String,
}
pub enum Item {
    Streaming {
        text: String,
    },
    Local {
        command: String,
        result: Option<String>,
        kind: Kind,
        details: Vec<(String, String)>,
    },
    Text {
        kind: Kind,
        text: String,
    },
    Tool {
        id: String,
        name: String,
        arguments: Value,
        summary: String,
        result: Option<Value>,
        change: Option<Box<crate::file_change::Change>>,
        last: Option<bool>,
        presentation: Presentation,
    },
    Collaboration {
        event: Value,
        expanded: bool,
    },
}
impl Item {
    pub fn is_visible(&self) -> bool {
        match self {
            Self::Streaming { text }
            | Self::Text {
                kind: Kind::Assistant,
                text,
            } => !text.trim().is_empty(),
            Self::Local { .. }
            | Self::Text {
                kind: Kind::Notice | Kind::Warning,
                ..
            } => false,
            _ => true,
        }
    }
}
pub struct State {
    pub items: Vec<Item>,
    pub editor: Editor,
    pub status: String,
    pub generation: u64,
    pub revision: u64,
    /// Visible conversation output, independent of presentation-only reflow.
    pub output_revision: u64,
    /// Assistant text already placed before an interleaved collaboration receipt.
    pub stream_prefix: Option<String>,
    pub tool_revision: u64,
    pub tool_details: tools::ToolDetails,
    pub width: usize,
    pub height: usize,
    pub effort: String,
    pub history: History,
    pub queue: Queue,
    pub suggestions: Suggestions,
    pub activity: Option<Activity>,
    pub notice: Option<Feedback>,
    pub copy_notice: Option<Feedback>,
    pub progress: Option<Feedback>,
    /// A retry temporarily overlays maintenance without replacing its clock.
    pub recovery: Option<Feedback>,
    /// How the last turn ended, until the next send.
    pub ending: Option<Feedback>,
    pub backend_failure: Option<(FailureReason, String)>,
    pub information: Option<Information>,
    pub selector: Option<super::selector::Selector>,
    pub processes: Option<super::processes::Panel>,
    pub collaboration: Vec<Value>,
    pub agents_panel: Option<super::agents::Panel>,
    /// The engine's latest projection for the footer; None renders no indicator.
    pub context: Option<crate::context::Status>,
    /// The recent conversations offered before the first prompt.
    pub opening: Opening,
    /// The quit guard is open: the background processes by command and id.
    pub quitting: Option<Vec<(String, String)>>,
}
impl Default for State {
    fn default() -> Self {
        Self {
            items: vec![],
            editor: Editor::default(),
            status: "Ready".into(),
            generation: 0,
            revision: 0,
            output_revision: 0,
            stream_prefix: None,
            tool_revision: 0,
            tool_details: tools::ToolDetails::default(),
            width: 80,
            height: 24,
            effort: "default".into(),
            history: History::default(),
            queue: Queue::default(),
            suggestions: Suggestions::default(),
            activity: None,
            notice: None,
            copy_notice: None,
            progress: None,
            recovery: None,
            ending: None,
            backend_failure: None,
            information: None,
            selector: None,
            processes: None,
            collaboration: Vec::new(),
            agents_panel: None,
            context: None,
            opening: Opening::default(),
            quitting: None,
        }
    }
}
impl State {
    pub fn message(&mut self, kind: Kind, text: &str) {
        if matches!(kind, Kind::Assistant | Kind::Notice) && text.trim().is_empty() {
            return;
        }
        self.close_tools();
        if matches!(kind, Kind::Notice | Kind::Warning | Kind::Error) {
            self.notify(Feedback::result(kind, text));
            return;
        }
        self.changed();
        self.output_changed();
        self.items.push(Item::Text {
            kind,
            text: text.into(),
        });
    }
    /// A failed turn keeps its diagnostic as the turn's last transcript row.
    pub fn diagnostic(&mut self, text: &str) {
        self.close_tools();
        self.changed();
        self.output_changed();
        self.items.push(Item::Text {
            kind: Kind::Error,
            text: text.into(),
        });
    }
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
                self.stream_prefix = None;
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
            Event::Context(status) => self.context = Some(status),
            Event::Generated { tokens } => {
                if let Some(activity) = &mut self.activity {
                    activity.generated += tokens;
                }
            }
            Event::Streaming { text } => {
                self.status = "Writing the response…".into();
                let text = self.stream_prefix.as_ref().and_then(|prefix| text.strip_prefix(prefix)).unwrap_or(&text).to_string();
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
                let text = self.stream_prefix.take().as_ref().and_then(|prefix| text.strip_prefix(prefix)).unwrap_or(&text).to_string();
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
                self.stream_prefix = None;
                if let Some(last) = self.open_branch() {
                    *last = Some(false);
                }
                self.status = "Working…".into();
                if let Some(activity) = &mut self.activity {
                    activity.tools += 1;
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
    pub fn collaboration_event(&mut self, event: Value) {
        if let Some(Item::Streaming { text }) = self.items.last() {
            let prefix = self.stream_prefix.get_or_insert_with(String::new);
            prefix.push_str(text);
            self.finish_stream();
        }
        self.changed();
        self.output_changed();
        self.items.push(Item::Collaboration {
            event,
            expanded: false,
        });
    }
    pub fn toggle_collaboration(&mut self) -> bool {
        let Some(index) = self
            .items
            .iter()
            .rposition(|item| matches!(item, Item::Collaboration { .. }))
        else {
            return false;
        };
        if let Item::Collaboration { expanded, .. } = &mut self.items[index] {
            *expanded = !*expanded;
        }
        self.changed();
        self.output_changed();
        self.tool_changed_from(index);
        true
    }
    fn open_branch(&mut self) -> Option<&mut Option<bool>> {
        self.items.iter_mut().rev().find_map(|item| match item {
            Item::Tool { last, .. } if last.is_none() => Some(last),
            _ => None,
        })
    }
    pub fn close_tools(&mut self) {
        if let Some(last) = self.open_branch() {
            *last = Some(true);
            self.changed();
        }
    }
    pub fn clear(&mut self) {
        self.finish_stream();
        self.close_tools();
        self.items.clear();
        self.stream_prefix = None;
        self.tool_details = tools::ToolDetails::default();
        self.generation = self.generation.wrapping_add(1);
        self.changed();
        self.status = "Ready".into();
        self.clear_notice();
        self.clear_copy_notice();
        self.ending = None;
        self.clear_progress();
        self.backend_failure = None;
        self.information = None;
        self.processes = None;
        self.agents_panel = None;
        self.collaboration.clear();
    }
    pub fn settled_len(&self) -> usize {
        self.items
            .iter()
            .position(|item| {
                matches!(
                    item,
                    Item::Streaming { .. }
                        | Item::Local { result: None, .. }
                        | Item::Tool { result: None, .. }
                        | Item::Tool { last: None, .. }
                )
            })
            .map(|index| rows::chapter_start(&self.items, index))
            .unwrap_or(self.items.len())
    }
    pub fn finish_stream(&mut self) {
        if let Some(Item::Streaming { text }) = self.items.last_mut() {
            let text = std::mem::take(text);
            *self.items.last_mut().unwrap() = Item::Text {
                kind: Kind::Assistant,
                text,
            };
            self.changed();
        }
    }
    pub fn settle_tools(&mut self, archive: &crate::export::Archive) {
        self.changed();
        let messages = archive.messages.lock().unwrap();
        let mut settled = Vec::new();
        for (index, item) in self.items.iter_mut().enumerate() {
            if let Item::Tool {
                id,
                name,
                summary,
                result,
                presentation,
                ..
            } = item
                && result.is_none()
            {
                let captured = messages
                    .iter()
                    .rev()
                    .find(|message| {
                        message.get("role").and_then(Value::as_str) == Some("tool")
                            && message
                                .get("tool_call_id")
                                .and_then(Value::as_str)
                                .is_some_and(|candidate| archive.redactor.text(candidate) == *id)
                    })
                    .and_then(|message| message.get("content").and_then(Value::as_str))
                    .and_then(|content| crate::json::parse(content).ok())
                    .unwrap_or_else(|| {
                        Value::object([(
                            "error",
                            Value::string("Turn ended before a result was received"),
                        )])
                    });
                *summary = archive
                    .redactor
                    .text(&crate::events::tool_summary(name, &captured));
                *result = Some(archive.redactor.value(&captured));
                presentation.finish();
                settled.push(index);
            }
        }
        drop(messages);
        if !settled.is_empty() {
            self.output_changed();
        }
        self.update_chapters(&settled);
    }
    pub fn changed(&mut self) {
        self.revision = self.revision.wrapping_add(1);
    }
    fn output_changed(&mut self) {
        self.output_revision = self.output_revision.wrapping_add(1);
    }
}

#[cfg(test)]
mod chapter_height_tests;
#[cfg(test)]
mod chapter_review_tests;
#[cfg(test)]
mod chapter_tests;
#[cfg(test)]
mod tests;
mod tools;
