use super::editor::Editor;
use crate::attachments::Prompt;
use crate::inbox::Inbox;
use std::collections::VecDeque;

#[derive(Default)]
pub struct History {
    prompts: VecDeque<Prompt>,
    position: Option<usize>,
    saved: Option<Editor>,
}

impl History {
    pub fn snapshot(&self) -> Vec<Prompt> {
        self.prompts.iter().cloned().collect()
    }

    pub fn withdraw(&mut self, prompts: &[Prompt]) {
        let count = prompts.iter().filter(|prompt| is_prompt(prompt)).count();
        self.prompts
            .truncate(self.prompts.len().saturating_sub(count));
        self.position = None;
    }

    pub fn restore(&mut self, prompts: Vec<Prompt>) {
        self.prompts = prompts
            .into_iter()
            .filter(is_prompt)
            .rev()
            .take(50)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        self.position = None;
        self.saved = None;
    }

    pub fn draft<'a>(&'a self, editor: &'a Editor) -> &'a Editor {
        self.saved.as_ref().unwrap_or(editor)
    }

    pub fn record(&mut self, prompt: impl Into<Prompt>) {
        let prompt = prompt.into();
        if !is_prompt(&prompt) {
            return;
        }
        self.prompts.push_back(prompt);
        if self.prompts.len() > 50 {
            self.prompts.pop_front();
            if let Some(position) = &mut self.position {
                // Positions are one-based; zero keeps an evicted recall just
                // before the oldest remaining entry during automatic sends.
                *position = position.saturating_sub(1);
            }
        }
    }

    pub fn edited(&mut self) {
        // A recalled prompt may now differ from history. Keep the original
        // composer parked until navigation returns to it or it is submitted.
        self.position = None;
    }

    pub fn submitted(&mut self, editor: &mut Editor) {
        self.position = None;
        if let Some(saved) = self.saved.take() {
            *editor = saved;
        } else {
            editor.take();
        }
    }

    pub fn is_browsing(&self) -> bool {
        self.saved.is_some()
    }

    pub fn has_edited_recall(&self) -> bool {
        self.saved.is_some() && self.position.is_none()
    }

    pub fn cancel(&mut self, editor: &mut Editor) -> bool {
        self.position = None;
        if let Some(saved) = self.saved.take() {
            *editor = saved;
            true
        } else {
            false
        }
    }

    pub fn navigate(&mut self, editor: &mut Editor, older: bool) {
        if self.prompts.is_empty() {
            return;
        }
        if older {
            let index = match self.position {
                Some(0 | 1) => return,
                Some(index) => index - 1,
                None => {
                    if self.saved.is_none() {
                        self.saved = Some(editor.clone());
                    }
                    self.prompts.len()
                }
            };
            editor.set(self.prompts[index - 1].clone());
            self.position = Some(index);
        } else if let Some(index) = self.position {
            if index < self.prompts.len() {
                editor.set(self.prompts[index].clone());
                self.position = Some(index + 1);
            } else {
                self.cancel(editor);
            }
        } else {
            self.cancel(editor);
        }
    }
}

fn is_prompt(prompt: &Prompt) -> bool {
    !prompt.attachments.is_empty()
        || (!prompt.text.trim().is_empty() && !prompt.text.trim_start().starts_with('/'))
}

/// Why a second message waits in the composer.
pub const PENDING: &str = "A draft is already pending · ↑ in an empty composer edits it";

/// The one pending draft. A queued draft lives in the inbox shared with the
/// agent, which takes ordinary messages while it works; a paused draft stays
/// local until the user takes it back.
#[derive(Default)]
pub struct Queue {
    inbox: Inbox,
    pub paused: Option<Editor>,
}

impl Queue {
    pub fn new(inbox: Inbox) -> Self {
        Self {
            inbox,
            ..Self::default()
        }
    }

    pub fn inbox(&self) -> &Inbox {
        &self.inbox
    }

    pub fn is_empty(&self) -> bool {
        self.paused.is_none() && self.inbox.with(|entries| entries.len() == 0)
    }

    /// The pending draft's display text, and whether it is paused.
    pub fn pending(&self) -> Option<(String, bool)> {
        if let Some(paused) = &self.paused {
            return Some((paused.display(), true));
        }
        self.inbox.with(|entries| {
            entries
                .prompts()
                .next()
                .map(|prompt| (prompt.display(), false))
        })
    }

    pub fn push(&mut self, prompt: impl Into<Prompt>) -> Result<(), String> {
        if !self.is_empty() {
            return Err(PENDING.into());
        }
        let prompt = prompt.into();
        self.inbox.with(|entries| entries.push(prompt)).map(drop)
    }

    /// The queued draft, for dispatch while the agent is idle.
    pub fn pop_front(&mut self) -> Option<Prompt> {
        self.inbox.with(|entries| entries.pop_front())
    }

    /// Takes the pending draft back; `None` once the agent has accepted it.
    pub fn take(&mut self) -> Option<Editor> {
        self.paused.take().or_else(|| {
            self.pop_front().map(|prompt| {
                let mut editor = Editor::default();
                editor.set(prompt);
                editor
            })
        })
    }

    /// Joins `draft` to the paused draft, after any text it already holds;
    /// alone, it stays paused with its caret.
    pub fn park(&mut self, draft: Editor) {
        if draft.prompt().is_empty() {
            return;
        }
        self.paused = Some(match self.paused.take() {
            Some(paused) => {
                let mut editor = Editor::default();
                editor.set(paused.prompt().join(draft.prompt()));
                editor
            }
            None => draft,
        });
    }

    /// Pauses a queued draft that has not joined the turn.
    pub fn pause(&mut self) {
        for prompt in self.inbox.with(|entries| entries.drain()) {
            let mut editor = Editor::default();
            editor.set(prompt);
            self.park(editor);
        }
    }

    /// Replaces the pending draft, as when another session opens.
    pub fn reset(&mut self, paused: Option<Editor>) {
        self.inbox.with(|entries| {
            entries.drain();
            entries.unacknowledged();
        });
        self.paused = paused;
    }

    /// Passes the queued and paused drafts to `save` while the inbox is
    /// locked. The agent accepts only prompts captured here, and an accepted
    /// message cannot reappear in a later capture.
    pub fn capture<R>(
        &self,
        save: impl FnOnce(Vec<Prompt>, Option<crate::sessions::Draft>) -> R,
    ) -> R {
        let paused = self.paused.as_ref().map(draft);
        self.inbox.with(|entries| save(entries.record(), paused))
    }

    /// Queued prompts, oldest first.
    #[cfg(test)]
    pub fn messages(&self) -> Vec<Prompt> {
        self.inbox
            .with(|entries| entries.prompts().cloned().collect())
    }
}

pub fn draft(editor: &Editor) -> crate::sessions::Draft {
    crate::sessions::Draft {
        text: editor.text.clone(),
        cursor: editor.cursor,
        attachments: editor.attachments.clone(),
    }
}

#[cfg(test)]
mod tests;
