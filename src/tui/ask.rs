use super::{App, editor::Editor, terminal::Key};
use crate::ask::{Draft, Request};

/// A question owns its answer editor. The chat editor is never swapped out.
pub struct Panel {
    pub editor: Editor,
    pub choice: Option<String>,
    pub selected: usize,
    pub choices_focused: bool,
}

impl Panel {
    pub fn new(request: &Request, saved: Option<&Draft>) -> Self {
        let saved = saved.filter(|draft| draft.id == request.id);
        let mut editor = Editor::default();
        if let Some(draft) = saved {
            editor.replace(draft.text.clone());
            editor.cursor = draft.cursor.min(editor.text.len());
            while !editor.text.is_char_boundary(editor.cursor) {
                editor.cursor -= 1;
            }
        }
        let choice = saved.and_then(|draft| draft.choice.clone()).filter(|id| {
            request
                .question
                .options
                .iter()
                .any(|option| &option.id == id)
        });
        let selected = choice
            .as_ref()
            .and_then(|id| {
                request
                    .question
                    .options
                    .iter()
                    .position(|option| &option.id == id)
            })
            .unwrap_or(0);
        Self {
            editor,
            choice,
            selected,
            choices_focused: false,
        }
    }

    pub fn draft(&self, id: &str) -> Draft {
        Draft {
            id: id.into(),
            text: self.editor.text.clone(),
            cursor: self.editor.cursor,
            choice: self.choice.clone(),
        }
    }
}

impl App {
    pub(super) fn refresh_question(&mut self) -> bool {
        let pending = self.question_observer.pending();
        if pending == self.state.question {
            return false;
        }
        match pending {
            Some(request) => {
                let saved = self
                    .persistence
                    .as_ref()
                    .and_then(|handle| handle.snapshot().input.question);
                self.state.ask_draft = saved.clone();
                self.state.ask_panel = Some(Panel::new(&request, saved.as_ref()));
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
            self.state.ask_panel = Some(Panel::new(request, saved.as_ref()));
        }
        self.state.todo_panel = None;
        self.state.agents_panel = None;
        self.state.processes = None;
        self.state.selector = None;
        self.state.information = None;
        self.state.suggestions.dismiss();
        self.state.changed();
    }

    pub(super) fn ask_text(&mut self, text: &str) {
        if let Some(panel) = &mut self.state.ask_panel {
            panel.choices_focused = false;
            if text.len() > (64_usize * 1024).saturating_sub(panel.editor.text.len())
                || !panel.editor.insert(text)
            {
                self.warn("Answer exceeds 64 KiB");
            }
            self.save_ask_draft();
            self.capture_input();
        }
    }

    pub(super) fn close_ask(&mut self) {
        self.save_ask_draft();
        self.state.ask_panel = None;
        self.capture_input();
    }

    fn save_ask_draft(&mut self) {
        if let (Some(panel), Some(request)) = (&self.state.ask_panel, &self.state.question) {
            self.state.ask_draft = Some(panel.draft(&request.id));
        }
    }

    pub(super) fn ask_scroll(&mut self, amount: i16) {
        if let Some(panel) = &mut self.state.ask_panel {
            let len = self
                .state
                .question
                .as_ref()
                .map_or(0, |r| r.question.options.len());
            if len > 0 {
                panel.choices_focused = true;
                panel.selected = if amount < 0 {
                    panel.selected.saturating_sub(1)
                } else {
                    (panel.selected + 1).min(len - 1)
                };
                panel.choice = self
                    .state
                    .question
                    .as_ref()
                    .map(|r| r.question.options[panel.selected].id.clone());
                self.save_ask_draft();
                self.capture_input();
            }
        }
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
        let Some(request) = self.state.question.as_ref() else {
            return;
        };
        if key.ctrl() && key.code == 68 && !key.alt() {
            let id = request.id.clone();
            if let Err(error) = self.question_observer.dismiss(&id) {
                self.warn(&error);
            } else {
                self.state.ask_panel = None;
                self.refresh_question();
                self.capture_input();
            }
            return;
        }
        let Some(panel) = self.state.ask_panel.as_mut() else {
            return;
        };
        if key.code == 9 && !key.ctrl() && !key.alt() {
            panel.choices_focused = !panel.choices_focused && !request.question.options.is_empty();
            return;
        }
        if panel.choices_focused
            && !request.question.options.is_empty()
            && matches!(key.code, 38 | 40)
        {
            let last = request.question.options.len() - 1;
            panel.selected = if key.code == 38 {
                panel.selected.saturating_sub(1)
            } else {
                (panel.selected + 1).min(last)
            };
            panel.choice = Some(request.question.options[panel.selected].id.clone());
            self.save_ask_draft();
            self.capture_input();
            return;
        }
        if key.code == 13 && !key.shift() && !key.alt() && !key.ctrl() {
            if panel.choice.is_none() && panel.editor.text.trim().is_empty() {
                self.warn("Choose an option or type an answer before submitting");
                return;
            }
            let (id, choice, text) = (
                request.id.clone(),
                panel.choice.clone(),
                panel.editor.text.clone(),
            );
            if let Err(error) = self.question_observer.answer(&id, choice, text) {
                self.warn(&error);
            } else {
                self.state.ask_panel = None;
                self.refresh_question();
                self.capture_input();
            }
            return;
        }
        if panel.choices_focused && key.code == 32 && !request.question.options.is_empty() {
            panel.choice = Some(request.question.options[panel.selected].id.clone());
            self.save_ask_draft();
            self.capture_input();
            return;
        }
        let before = (panel.editor.text.clone(), panel.editor.cursor);
        if panel.editor.text.len() >= 64 * 1024
            && (key.code == 13 && (key.shift() || key.alt()) || key.ctrl() && key.code == 74)
        {
            self.warn("Answer exceeds 64 KiB");
            return;
        }
        panel.editor.key(key);
        if (panel.editor.text.clone(), panel.editor.cursor) != before {
            self.save_ask_draft();
            self.capture_input();
        }
    }
}
