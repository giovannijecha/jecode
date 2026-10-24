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

/// A pending draft: a queued prompt by its inbox entry, or a paused draft.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Slot {
    Queued(u64),
    Paused(usize),
}

/// Queued prompts live in the inbox shared with the agent, which takes
/// ordinary messages while it works. Paused drafts stay local until sent.
#[derive(Default)]
pub struct Queue {
    inbox: Inbox,
    pub paused: Vec<Editor>,
    editing: Option<Editing>,
}

struct Editing {
    slot: Slot,
    previous: Editor,
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

    pub fn queued(&self) -> usize {
        self.inbox.with(|entries| entries.len())
    }

    pub fn len(&self) -> usize {
        self.queued() + self.paused.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn is_editing(&self) -> bool {
        self.editing.is_some()
    }

    pub fn editing(&self) -> Option<Slot> {
        self.editing.as_ref().map(|editing| editing.slot)
    }

    /// The combined position of the draft being edited.
    pub fn edit_index(&self) -> Option<usize> {
        self.editing().and_then(|slot| self.position(slot))
    }

    pub fn position(&self, slot: Slot) -> Option<usize> {
        match slot {
            Slot::Queued(id) => self.inbox.with(|entries| entries.position(id)),
            Slot::Paused(index) => (index < self.paused.len()).then(|| self.queued() + index),
        }
    }

    /// Queued prompts, then paused drafts, with their display text.
    pub fn drafts(&self) -> Vec<(Slot, String)> {
        let mut drafts: Vec<_> = self.inbox.with(|entries| {
            entries
                .prompts()
                .map(|(id, prompt)| (Slot::Queued(id), prompt.display()))
                .collect()
        });
        drafts.extend(
            self.paused
                .iter()
                .enumerate()
                .map(|(index, draft)| (Slot::Paused(index), draft.display())),
        );
        drafts
    }

    #[cfg(test)]
    pub fn slot(&self, index: usize) -> Option<Slot> {
        self.drafts().get(index).map(|(slot, _)| *slot)
    }

    pub fn draft<'a>(&'a self, editor: &'a Editor) -> &'a Editor {
        self.editing
            .as_ref()
            .map_or(editor, |editing| &editing.previous)
    }

    pub fn push(&mut self, prompt: impl Into<Prompt>) -> Result<(), String> {
        let prompt = prompt.into();
        self.inbox.update(|entries| entries.push(prompt)).map(drop)
    }

    /// The oldest queued prompt, for dispatch while the agent is idle.
    pub fn pop_front(&mut self) -> Option<Prompt> {
        if self.is_editing() {
            return None;
        }
        self.inbox.update(|entries| entries.pop_front())
    }

    pub fn begin_edit(&mut self, slot: Slot, editor: &mut Editor) -> Result<(), String> {
        if self.editing.is_some() {
            return Err("Finish the current queued edit first.".into());
        }
        let previous = editor.clone();
        match slot {
            Slot::Queued(id) => {
                // A held entry stays queued but cannot join the turn until the edit ends.
                let prompt = self
                    .inbox
                    .update(|entries| entries.hold(id))
                    .ok_or("This message was already sent.")?;
                editor.set(prompt);
            }
            Slot::Paused(index) => {
                *editor = self
                    .paused
                    .get(index)
                    .ok_or("No queued draft at that position.")?
                    .clone();
            }
        }
        self.editing = Some(Editing { slot, previous });
        Ok(())
    }

    pub fn save_edit(&mut self, editor: &mut Editor) -> bool {
        let Some(editing) = self.editing.take() else {
            return false;
        };
        match editing.slot {
            Slot::Queued(_) => {
                let prompt = editor.prompt();
                self.inbox.update(|entries| entries.release(Some(prompt)));
            }
            Slot::Paused(index) => self.paused[index] = editor.clone(),
        }
        *editor = editing.previous;
        true
    }

    pub fn cancel_edit(&mut self, editor: &mut Editor) -> bool {
        let Some(editing) = self.editing.take() else {
            return false;
        };
        if matches!(editing.slot, Slot::Queued(_)) {
            self.inbox.update(|entries| entries.release(None));
        }
        *editor = editing.previous;
        true
    }

    pub fn discard(&mut self, slot: Slot) -> bool {
        self.take(slot).is_some()
    }

    /// Removes a draft that has not been sent; `None` if it already was.
    pub fn take(&mut self, slot: Slot) -> Option<Prompt> {
        if self.is_editing() {
            return None;
        }
        match slot {
            Slot::Queued(id) => self.inbox.update(|entries| entries.remove(id)),
            Slot::Paused(index) => {
                (index < self.paused.len()).then(|| self.paused.remove(index).take())
            }
        }
    }

    /// Turns every queued prompt into a leading paused draft.
    pub fn pause(&mut self) {
        let (prompts, held) = self.inbox.update(|entries| entries.drain());
        let count = prompts.len();
        if count == 0 {
            return;
        }
        let mut paused: Vec<_> = prompts
            .into_iter()
            .map(|prompt| {
                let mut editor = Editor::default();
                editor.set(prompt);
                editor
            })
            .collect();
        paused.append(&mut self.paused);
        self.paused = paused;
        // Combined positions are unchanged when queued entries lead the paused ones.
        if let Some(editing) = &mut self.editing {
            editing.slot = match editing.slot {
                Slot::Queued(_) => Slot::Paused(held.unwrap_or(0)),
                Slot::Paused(index) => Slot::Paused(index + count),
            };
        }
    }

    /// Replaces every draft, as when another session opens.
    pub fn reset(&mut self, paused: Vec<Editor>) {
        self.inbox.update(|entries| {
            entries.drain();
            entries.unacknowledged();
        });
        self.editing = None;
        self.paused = paused;
    }

    /// Passes queued and paused drafts, including an unsaved edit, to `save`
    /// while the inbox is locked. The agent accepts only prompts captured
    /// here, and an accepted message cannot reappear in a later capture.
    pub fn capture<R>(
        &self,
        editor: &Editor,
        save: impl FnOnce(Vec<Prompt>, Vec<crate::sessions::Draft>) -> R,
    ) -> R {
        let editing = self.editing();
        let mut paused: Vec<_> = self.paused.iter().map(draft).collect();
        if let Some(Slot::Paused(index)) = editing {
            paused[index] = draft(editor);
        }
        self.inbox.with(|entries| {
            if let Some(Slot::Queued(_)) = editing {
                entries.revise(editor.prompt());
            }
            save(entries.record(), paused)
        })
    }

    #[cfg(test)]
    pub fn snapshot(&self, editor: &Editor) -> (Vec<Prompt>, Vec<crate::sessions::Draft>) {
        self.capture(editor, |queued, paused| (queued, paused))
    }

    /// Queued prompts, oldest first.
    #[cfg(test)]
    pub fn messages(&self) -> Vec<Prompt> {
        self.inbox.with(|entries| {
            entries
                .prompts()
                .map(|(_, prompt)| prompt.clone())
                .collect()
        })
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
