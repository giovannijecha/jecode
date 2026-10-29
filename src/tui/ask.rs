//! Ask user in the composer's place: keys move the cursor, Space and Enter
//! choose, typing fills `Other`, and the last Enter sends every answer.

use super::{App, terminal::Key, viewport::Scroll};
use crate::ask::Request;

mod panel;
pub use panel::{Entry, Panel};

#[cfg(test)]
mod tests;

impl App {
    pub(super) fn refresh_question(&mut self) -> bool {
        let pending = self.question_observer.pending();
        if pending == self.state.question {
            return false;
        }
        match pending {
            Some(request) => {
                // A new question closes any menu over it, as /ask does.
                if self.state.question.is_none() {
                    self.make_room_for_ask();
                }
                let saved = self
                    .persistence
                    .as_ref()
                    .and_then(|handle| handle.snapshot().input.question);
                self.state.ask_draft = saved.clone();
                let mut panel = Panel::new(&request, saved.as_ref());
                if !self.state.editor.text.is_empty() {
                    panel.protect_arrival(&request);
                }
                self.state.ask_panel = Some(panel);
                self.state.question = Some(request);
            }
            None => {
                self.state.ask_panel = None;
                self.state.ask_draft = None;
                self.state.question = None;
            }
        }
        self.state.changed();
        true
    }

    pub(super) fn open_ask(&mut self) {
        if self.state.question.is_none() {
            self.warn("No question is waiting for an answer");
            return;
        }
        if self.state.ask_panel.is_none() {
            let request = self.state.question.as_ref().unwrap();
            let saved = self.state.ask_draft.clone().or_else(|| {
                self.persistence
                    .as_ref()
                    .and_then(|handle| handle.snapshot().input.question)
            });
            let mut panel = Panel::new(request, saved.as_ref());
            if !self.state.editor.text.is_empty() {
                panel.protect_arrival(request);
            }
            self.state.ask_panel = Some(panel);
        }
        self.make_room_for_ask();
    }

    /// Closes every panel and menu, each the way Esc would, so the question
    /// takes the composer's place. The quit confirmation stays above it.
    pub(super) fn make_room_for_ask(&mut self) {
        // Closing a submenu can return to its parent; settle within a few steps.
        for _ in 0..8 {
            if self.state.selector.is_none() {
                break;
            }
            self.close_selector();
        }
        self.state.selector = None;
        self.state.todo_panel = None;
        self.state.agents_panel = None;
        self.state.processes = None;
        self.state.information = None;
        self.state.suggestions.dismiss();
        self.state.changed();
    }

    /// Space selects an option; any other text, or Space in the typed
    /// words, goes to `Other`.
    pub(super) fn ask_text(&mut self, text: &str, pasted: bool) {
        let (Some(panel), Some(request)) = (&mut self.state.ask_panel, &self.state.question) else {
            return;
        };
        let question = &request.questions[panel.step];
        if !pasted
            && panel.entry().row == usize::MAX
            && text.len() == 1
            && let Some(index) = text
                .chars()
                .next()
                .and_then(|ch| ch.to_digit(10))
                .and_then(|number| number.checked_sub(1))
                .map(|number| number as usize)
            && index <= question.options.len()
        {
            let multiple = question.is_multiple();
            let option = index < question.options.len();
            let request = request.clone();
            panel.entries[panel.step].row = index;
            if option {
                panel.select(question);
            }
            self.ask_changed();
            if !multiple && option {
                self.ask_enter(&request);
            }
            return;
        }
        let selected = text == " " && panel.select(question);
        if !selected && !panel.type_text(question, text) {
            self.warn("Answer exceeds 64 KiB");
        }
        self.ask_changed();
    }

    pub(super) fn close_ask(&mut self) {
        self.save_ask_draft();
        self.state.ask_panel = None;
        self.capture_input();
    }

    fn save_ask_draft(&mut self) {
        if let (Some(panel), Some(request)) = (&self.state.ask_panel, &self.state.question) {
            self.state.ask_draft = Some(panel.draft(request));
        }
    }

    fn ask_changed(&mut self) {
        self.save_ask_draft();
        self.capture_input();
        self.state.changed();
    }

    /// The wheel scrolls a block taller than its room, else the conversation.
    pub(super) fn ask_scroll(&mut self, amount: i16) {
        if !self.scroll_ask(isize::from(amount)) {
            self.renderer.scroll(Scroll::Rows(amount.into()));
        }
    }

    fn scroll_ask(&mut self, amount: isize) -> bool {
        let Some(window) = super::view::ask_window(&self.state) else {
            return false;
        };
        let Some(panel) = self.state.ask_panel.as_mut() else {
            return false;
        };
        if window.total <= window.room {
            return false;
        }
        let last = window.total - window.room;
        panel.top = Some(window.start.saturating_add_signed(amount).min(last));
        self.state.changed();
        true
    }

    pub(super) fn ask_key(&mut self, key: Key) {
        if key.ctrl() && key.code == 67 {
            self.stop();
            return;
        }
        if key.code == 27 {
            self.close_ask();
            return;
        }
        let Some(request) = self.state.question.clone() else {
            return;
        };
        if key.ctrl() && key.code == 68 && !key.alt() {
            if let Err(error) = self.question_observer.dismiss(&request.id) {
                self.warn(&error);
            } else {
                self.state.ask_panel = None;
                self.refresh_question();
                self.capture_input();
            }
            return;
        }
        let window = super::view::ask_window(&self.state);
        let Some(panel) = self.state.ask_panel.as_mut() else {
            return;
        };
        let question = &request.questions[panel.step];
        let typing = panel.entry().in_text(question);
        // ←→ move the caret through typed words, else between questions.
        let words = typing && !panel.entry().editor.text.is_empty();
        let plain = !key.ctrl() && !key.alt() && !key.shift();
        match key.code {
            13 if plain => return self.ask_enter(&request),
            9 if !key.ctrl() && !key.alt() => {
                if panel.switch(&request, !key.shift()) {
                    self.ask_changed();
                }
                return;
            }
            37 | 39 if plain && !words => {
                if panel.switch(&request, key.code == 39) {
                    self.ask_changed();
                }
                return;
            }
            38 | 40 if plain => {
                let up = key.code == 38;
                let moved = if question.options.is_empty() {
                    let columns = window.map_or(1, |window| window.text);
                    panel.entries[panel.step].editor.vertical(up, columns)
                } else {
                    panel.move_row(question, up)
                };
                if moved {
                    self.ask_changed();
                }
                return;
            }
            33 | 34 if !key.ctrl() && !key.alt() => {
                let page = window.map_or(1, |window| window.room.saturating_sub(1).max(1));
                let amount = if key.code == 33 { -1 } else { 1 } * page as isize;
                if !self.scroll_ask(amount) {
                    self.renderer.scroll(Scroll::Page(key.code == 33));
                }
                return;
            }
            _ => {}
        }
        if !typing {
            return;
        }
        let before = panel.entry().editor.clone();
        if !panel.edit(question, key) {
            self.warn("Answer exceeds 64 KiB");
            return;
        }
        if panel.entry().editor != before {
            self.ask_changed();
        }
    }

    /// Enter chooses the cursor row of a single choice, or keeps what is
    /// selected, then moves to the next question; the last sends them all.
    fn ask_enter(&mut self, request: &Request) {
        let Some(panel) = self.state.ask_panel.as_mut() else {
            return;
        };
        if panel.entry().row == usize::MAX {
            return;
        }
        let question = &request.questions[panel.step];
        let chosen = if question.is_multiple() {
            true
        } else {
            panel.choose(question)
        };
        if !chosen || panel.entry().answer(question).is_none() {
            self.ask_changed();
            self.warn(if question.is_multiple() {
                "Select an answer with Space, or type your own"
            } else if question.options.is_empty() {
                "Type your answer, or press Ctrl+D to dismiss"
            } else {
                "Type your own answer, or choose an option"
            });
            return;
        }
        if panel.step + 1 < request.questions.len() {
            panel.switch(request, true);
            self.ask_changed();
            return;
        }
        let answers = match panel.answers(request) {
            Ok(answers) => answers,
            Err(index) => {
                panel.step = index;
                panel.top = None;
                let title = request.questions[index].title.clone();
                self.ask_changed();
                self.warn(&format!("Answer {title} before sending"));
                return;
            }
        };
        self.ask_changed();
        if let Err(error) = self.question_observer.answer(&request.id, answers) {
            self.warn(&error);
        } else {
            self.state.ask_panel = None;
            self.refresh_question();
            self.capture_input();
        }
    }
}
