//! Bounded in-memory navigation over user prompts; the unsent draft is a snapshot.
use super::editor::Editor;

const CAPACITY: usize = crate::session::MAX_RECALLED_PROMPTS;

#[derive(Default)]
pub struct PromptHistory {
    entries: Vec<String>,
    position: Option<usize>,
    draft: Option<(Editor, bool)>,
}
impl PromptHistory {
    pub fn load(&mut self, prompts: Vec<String>) {
        self.entries = prompts.into_iter().rev().take(CAPACITY).collect();
        self.entries.reverse();
        self.position = None;
        self.draft = None;
    }
    pub fn record(&mut self, prompt: &str) {
        self.entries.push(prompt.to_owned());
        if self.entries.len() > CAPACITY {
            self.entries.remove(0);
        }
        self.position = None;
        self.draft = None;
    }
    pub fn previous(&mut self, editor: &mut Editor, literal: &mut bool) {
        if self.entries.is_empty() {
            return;
        }
        let index = match self.position {
            Some(0) => return,
            Some(index) => index - 1,
            None => {
                self.draft = Some((editor.clone(), *literal));
                self.entries.len() - 1
            }
        };
        self.position = Some(index);
        editor.replace(&self.entries[index]);
        *literal = editor.text.starts_with('/');
    }
    pub fn next(&mut self, editor: &mut Editor, literal: &mut bool) {
        let Some(index) = self.position else {
            return;
        };
        if index + 1 < self.entries.len() {
            self.position = Some(index + 1);
            editor.replace(&self.entries[index + 1]);
            *literal = editor.text.starts_with('/');
        } else {
            self.position = None;
            if let Some((saved, was_literal)) = self.draft.take() {
                let columns = editor.columns();
                *editor = saved;
                editor.set_columns(columns);
                *literal = was_literal;
            }
        }
    }
}
