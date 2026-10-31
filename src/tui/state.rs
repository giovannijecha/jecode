use super::{
    activity::Activity,
    cards::Presentation,
    drafts::{History, Queue},
    editor::Editor,
    feedback::Feedback,
    information::Information,
    suggestions::Suggestions,
};
use crate::backend::FailureReason;
use crate::json::Value;

mod agents;
mod events;
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
    /// Available personal skills at launch.
    pub skills: usize,
    /// Whether the working directory has a root AGENTS.md.
    pub agents: bool,
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
    /// When each running collaborator was first seen running, for `/agents`.
    pub agent_clocks: std::collections::HashMap<String, std::time::Instant>,
    pub agents_panel: Option<super::agents::Panel>,
    pub question: Option<crate::ask::Request>,
    pub ask_panel: Option<super::ask::Panel>,
    pub ask_draft: Option<crate::ask::Draft>,
    /// This conversation's level, beside the draft (B12.3).
    pub permission_level: crate::permissions::Level,
    /// The request in the composer's place (B12.4).
    pub permission: Option<super::permission::Pending>,
    pub todo_plan: Option<crate::todo::Plan>,
    pub todo_panel: Option<super::todo::Panel>,
    /// Decides which todo_list calls create a plan; only those have a row.
    pub todo_calls: super::todo::Calls,
    /// The latest provider usage snapshot for the footer; None renders no indicator.
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
            agent_clocks: std::collections::HashMap::new(),
            agents_panel: None,
            question: None,
            ask_panel: None,
            ask_draft: None,
            permission_level: Default::default(),
            permission: None,
            todo_plan: None,
            todo_panel: None,
            todo_calls: Default::default(),
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
        self.ending = None;
        self.clear_progress();
        self.backend_failure = None;
        self.information = None;
        self.processes = None;
        self.agents_panel = None;
        self.question = None;
        self.ask_panel = None;
        self.ask_draft = None;
        self.todo_plan = None;
        self.todo_panel = None;
        self.todo_calls = Default::default();
        self.collaboration.clear();
        self.agent_clocks.clear();
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
