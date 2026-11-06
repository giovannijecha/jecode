use super::App;
use crate::{
    agent::{Agent, TurnInput},
    attachments::Prompt,
    events::Event,
};

pub(super) struct Submission {
    input: TurnInput,
    prompts: Vec<Prompt>,
    items: Vec<usize>,
    committed: bool,
    streaming: bool,
}

impl Submission {
    pub fn new(input: TurnInput) -> Self {
        Self {
            input,
            prompts: vec![],
            items: vec![],
            committed: false,
            streaming: false,
        }
    }

    pub fn accepted(&mut self, prompt: Prompt, item: usize) {
        self.prompts.push(prompt);
        self.items.push(item);
    }

    pub fn observe(&mut self, event: &Event) {
        match event {
            Event::Streaming { text } if !text.trim().is_empty() => self.streaming = true,
            Event::Message { text } => {
                self.committed |= !text.trim().is_empty();
                self.streaming = false;
            }
            Event::ToolStarted { .. } => self.committed = true,
            Event::Recovering { .. } | Event::RequestDiscarded => self.streaming = false,
            _ => {}
        }
    }

    fn has_output(&self) -> bool {
        self.committed || self.streaming
    }
}

impl App {
    /// Restore the unanswered requests as one draft and park any newer
    /// composer text. A response or a started tool makes the turn durable.
    pub(super) fn return_unanswered(&mut self, agent: &mut Agent) -> bool {
        let Some(submission) = self.submission.take() else {
            return false;
        };
        if submission.has_output() || !agent.return_input(submission.input) {
            return false;
        }
        self.park_composer_for_restore();
        self.state.history.withdraw(&submission.prompts);
        let returned = submission.prompts.into_iter().reduce(Prompt::join);
        self.state.editor.set(returned.unwrap());
        if let Some(index) = &mut self.pending_command {
            *index -= submission
                .items
                .iter()
                .filter(|&&item| item < *index)
                .count();
        }
        let mut index = 0;
        self.state.items.retain(|_| {
            let keep = submission.items.binary_search(&index).is_err();
            index += 1;
            keep
        });
        self.state.reindex_chapters();
        self.state.reset_tool_details();
        self.state.generation = self.state.generation.wrapping_add(1);
        self.state.changed();
        self.edited();
        self.capture_input();
        match agent.save_session() {
            Ok(()) => self.save_error = None,
            Err(error) => {
                self.save_error = Some(agent.redact(&format!(
                    "Autosave failed: {error}. Session kept in memory."
                )));
            }
        }
        true
    }

    /// Pauses a queued draft that has not joined the turn. An empty
    /// composer takes the paused draft back at once.
    pub(super) fn pause_draft(&mut self) {
        self.state.queue.pause();
        if !self.state.history.is_browsing()
            && self.state.editor.prompt().is_empty()
            && let Some(draft) = self.state.queue.paused.take()
        {
            self.state.editor = draft;
        }
    }

    fn park_composer(&mut self) {
        let draft = std::mem::take(&mut self.state.editor);
        self.state.queue.park(draft);
    }

    /// The composer's draft, taken away from this conversation. An edited recall stays
    /// behind as its paused draft, as when a request comes back.
    pub(super) fn take_composer(&mut self) -> super::editor::Editor {
        if self.state.history.has_edited_recall() {
            self.park_composer();
        }
        self.state.history.cancel(&mut self.state.editor);
        std::mem::take(&mut self.state.editor)
    }

    fn park_composer_for_restore(&mut self) {
        if self.state.history.has_edited_recall() {
            self.park_composer();
        }
        self.state.history.cancel(&mut self.state.editor);
        self.park_composer();
    }

    pub(super) fn restore_failed_start(&mut self, prompt: Prompt) {
        self.pause_draft();
        self.park_composer_for_restore();
        self.state.editor.set(prompt);
        self.edited();
        self.capture_input();
    }
}

/// A draft that came along goes first, the conversation's own joins below it, and the
/// caret stays where it was written.
pub(super) fn carry_draft(editor: &mut super::editor::Editor, carried: super::editor::Editor) {
    let saved = editor.prompt();
    if saved.is_empty() {
        *editor = carried;
    } else if !carried.prompt().is_empty() {
        editor.set(carried.prompt().join(saved));
        editor.cursor = carried.cursor;
    }
}

#[cfg(test)]
#[path = "recovery_tests.rs"]
mod tests;
