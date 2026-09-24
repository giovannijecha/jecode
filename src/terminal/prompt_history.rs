//! Bounded in-memory navigation over user prompts; the unsent draft is a snapshot.
use super::editor::Editor;

const CAPACITY: usize = crate::session::MAX_RECALLED_PROMPTS;

#[derive(Default)]
pub struct PromptHistory {
    entries: Vec<String>,
    first_id: usize,
    position: Option<usize>,
    draft: Option<(Editor, bool)>,
}
#[derive(Clone)]
pub(super) struct Navigation {
    position: Option<usize>,
    draft: Option<(Editor, bool)>,
}
impl PromptHistory {
    pub(super) fn navigation(&self) -> Navigation {
        Navigation {
            position: self.position,
            draft: self.draft.clone(),
        }
    }
    pub(super) fn restore_navigation(&mut self, saved: Navigation) {
        self.position = saved.position;
        self.draft = saved.draft;
    }
    pub fn load(&mut self, prompts: Vec<String>) {
        self.entries = prompts.into_iter().rev().take(CAPACITY).collect();
        self.entries.reverse();
        self.first_id = 0;
        self.position = None;
        self.draft = None;
    }
    pub fn record(&mut self, prompt: &str) {
        self.push(prompt);
        self.position = None;
        self.draft = None;
    }
    /// Canonical turns can arrive while browsing. Keep the shown entry and the
    /// saved draft; absolute IDs also survive eviction of the oldest entry.
    pub fn record_new_turn(&mut self, prompt: &str) {
        self.push(prompt);
    }
    fn push(&mut self, prompt: &str) {
        self.entries.push(prompt.to_owned());
        if self.entries.len() > CAPACITY {
            self.entries.remove(0);
            self.first_id += 1;
        }
    }
    pub fn previous(&mut self, editor: &mut Editor, literal: &mut bool) {
        if self.entries.is_empty() {
            return;
        }
        let index = match self.position {
            Some(index) if index <= self.first_id => return,
            Some(index) => index - 1,
            None => {
                self.draft = Some((editor.clone(), *literal));
                self.first_id + self.entries.len() - 1
            }
        };
        self.position = Some(index);
        editor.replace(&self.entries[index - self.first_id]);
        *literal = editor.text.starts_with('/');
    }
    pub fn next(&mut self, editor: &mut Editor, literal: &mut bool) {
        let Some(index) = self.position else {
            return;
        };
        let next = (index + 1).max(self.first_id);
        if next < self.first_id + self.entries.len() {
            self.position = Some(next);
            editor.replace(&self.entries[next - self.first_id]);
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
