use super::{
    activity::Activity,
    cards::Presentation,
    drafts::{History, Queue},
    editor::Editor,
    feedback::Feedback,
    information::Information,
    suggestions::Suggestions,
};
use crate::events::Event;
use crate::json::Value;

pub(super) mod rows;

#[derive(Clone, Copy)]
pub enum Kind {
    User,
    Assistant,
    Notice,
    Warning,
    Error,
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
                kind: Kind::Notice | Kind::Warning | Kind::Error,
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
    pub information: Option<Information>,
    pub selector: Option<super::selector::Selector>,
    pub processes: Option<super::processes::Panel>,
    /// The engine's latest projection for the footer; None renders no indicator.
    pub context: Option<crate::context::Status>,
}
impl Default for State {
    fn default() -> Self {
        Self {
            items: vec![],
            editor: Editor::default(),
            status: "Ready".into(),
            generation: 0,
            revision: 0,
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
            information: None,
            selector: None,
            processes: None,
            context: None,
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
        self.items.push(Item::Text {
            kind,
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
        ) {
            self.changed();
        }
        match event {
            Event::Recovering {
                attempt,
                delay,
                error,
            } => {
                if matches!(self.items.last(), Some(Item::Streaming { .. })) {
                    self.items.pop();
                    self.changed();
                }
                self.status = "Waiting for connection…".into();
                // The rule counts down to the next attempt.
                let subject = if error.contains("HTTP 429") {
                    "Rate limited"
                } else {
                    "Reconnecting"
                };
                self.recovery = Some(
                    Feedback::progress(
                        Kind::Warning,
                        format!("{subject} · attempt {attempt} · {error}"),
                    )
                    .until(std::time::Instant::now() + delay),
                );
            }
            Event::RecoveryFinished => {
                self.recovery = None;
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
            Event::Streaming { text } => {
                self.status = "Writing the response…".into();
                // Updates contain the entire response so far. Its leading whitespace
                // remains available when visible content arrives in a later update.
                if text.trim().is_empty() {
                    return;
                }
                if let Some(Item::Streaming { text: current }) = self.items.last_mut() {
                    *current = text;
                } else {
                    self.close_tools();
                    self.items.push(Item::Streaming { text });
                }
            }
            Event::Message { text } => {
                if matches!(self.items.last(), Some(Item::Streaming { .. })) {
                    *self.items.last_mut().unwrap() = Item::Text {
                        kind: Kind::Assistant,
                        text,
                    };
                } else {
                    self.message(Kind::Assistant, &text);
                }
            }
            Event::ToolStarted {
                id,
                name,
                arguments,
            } => {
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
                    presentation: Presentation::live(),
                });
            }
            Event::ToolNote { id, text } => {
                if let Some(Item::Tool { presentation, .. }) = self.items.iter_mut().rev().find(
                    |item| matches!(item, Item::Tool { id: current, result: None, .. } if *current == id),
                ) {
                    presentation.note = Some(text);
                }
            }
            Event::ToolChanged { id, change } => {
                if let Some(Item::Tool { change: captured, .. }) = self.items.iter_mut().rev().find(|item| matches!(item, Item::Tool { id: current, result: None, .. } if *current == id)) {
                    *captured = Some(Box::new(change));
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
                    self.join_reads(&[index]);
                }
            }
        }
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
        self.tool_details = tools::ToolDetails::default();
        self.generation = self.generation.wrapping_add(1);
        self.changed();
        self.status = "Ready".into();
        self.clear_notice();
        self.clear_copy_notice();
        self.clear_progress();
        self.information = None;
        self.processes = None;
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
        self.join_reads(&settled);
    }
    pub fn changed(&mut self) {
        self.revision = self.revision.wrapping_add(1);
    }
}

#[cfg(test)]
mod tests;
mod tools;
