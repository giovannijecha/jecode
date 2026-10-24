use super::{App, editor::Editor};
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
    /// Restore the first request, park other accepted input, and preserve any
    /// new draft or edit. A response or a started tool makes the turn durable.
    pub(super) fn return_unanswered(&mut self, agent: &mut Agent) -> bool {
        let Some(submission) = self.submission.take() else {
            return false;
        };
        if submission.has_output() || !agent.return_input(submission.input) {
            return false;
        }
        self.park_composer_for_restore();
        self.state.history.withdraw(&submission.prompts);
        let mut prompts = submission.prompts.into_iter();
        self.state.editor.set(prompts.next().unwrap());
        let mut paused: Vec<_> = prompts
            .map(|prompt| {
                let mut editor = Editor::default();
                editor.set(prompt);
                editor
            })
            .collect();
        paused.append(&mut self.state.queue.paused);
        self.state.queue.paused = paused;
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
        self.state.reset_tool_details();
        self.state.generation = self.state.generation.wrapping_add(1);
        self.state.changed();
        self.refresh_draft_menu();
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

    fn park_composer(&mut self) {
        if !self.state.editor.text.is_empty() {
            self.state
                .queue
                .paused
                .push(std::mem::take(&mut self.state.editor));
        }
    }

    fn park_composer_for_restore(&mut self) {
        if self.state.queue.is_editing() {
            self.state.queue.save_edit(&mut self.state.editor);
        }
        if self.state.history.has_edited_recall() {
            self.park_composer();
        }
        self.state.history.cancel(&mut self.state.editor);
        self.park_composer();
    }

    pub(super) fn restore_failed_start(&mut self, prompt: Prompt) {
        self.state.queue.pause();
        self.park_composer_for_restore();
        self.state.editor.set(prompt);
        self.edited();
        self.capture_input();
    }
}

#[cfg(test)]
#[path = "recovery_tests.rs"]
mod tests;
