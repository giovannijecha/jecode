use super::editor::Editor;
use crate::attachments::Prompt;
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

#[derive(Default)]
pub struct Queue {
    pub messages: VecDeque<Prompt>,
    pub paused: Vec<Editor>,
    editing: Option<Editing>,
}

struct Editing {
    index: usize,
    previous: Editor,
}

impl Queue {
    pub fn len(&self) -> usize {
        self.messages.len() + self.paused.len()
    }

    pub fn is_empty(&self) -> bool {
        self.messages.is_empty() && self.paused.is_empty()
    }

    pub fn is_editing(&self) -> bool {
        self.editing.is_some()
    }

    pub fn edit_index(&self) -> Option<usize> {
        self.editing.as_ref().map(|editing| editing.index)
    }

    pub fn draft<'a>(&'a self, editor: &'a Editor) -> &'a Editor {
        self.editing
            .as_ref()
            .map_or(editor, |editing| &editing.previous)
    }

    pub fn push(&mut self, prompt: impl Into<Prompt>) -> Result<(), String> {
        let prompt = prompt.into();
        if self.messages.len() == 8 {
            return Err("Queue is full (8 messages). Your draft was kept.".into());
        }
        if let Some(editing) = &mut self.editing
            && editing.index >= self.messages.len()
        {
            editing.index += 1;
        }
        self.messages.push_back(prompt);
        Ok(())
    }

    pub fn begin_edit(&mut self, index: usize, editor: &mut Editor) -> Result<(), String> {
        if self.editing.is_some() {
            return Err("Finish the current queued edit first.".into());
        }
        let Some(prompt) = (index < self.messages.len())
            .then(|| self.messages[index].clone())
            .or_else(|| {
                self.paused
                    .get(index - self.messages.len())
                    .map(Editor::prompt)
            })
        else {
            return Err("No queued draft at that position.".into());
        };
        let previous = editor.clone();
        if index < self.messages.len() {
            editor.set(prompt);
        } else {
            *editor = self.paused[index - self.messages.len()].clone();
        }
        self.editing = Some(Editing { index, previous });
        Ok(())
    }

    pub fn save_edit(&mut self, editor: &mut Editor) -> bool {
        let Some(editing) = self.editing.take() else {
            return false;
        };
        if editing.index < self.messages.len() {
            self.messages[editing.index] = editor.prompt();
        } else {
            self.paused[editing.index - self.messages.len()] = editor.clone();
        }
        *editor = editing.previous;
        true
    }

    pub fn cancel_edit(&mut self, editor: &mut Editor) -> bool {
        let Some(editing) = self.editing.take() else {
            return false;
        };
        *editor = editing.previous;
        true
    }

    pub fn discard(&mut self, index: usize) -> bool {
        if self.is_editing() {
            return false;
        }
        if index < self.messages.len() {
            self.messages.remove(index).is_some()
        } else {
            let index = index - self.messages.len();
            if index < self.paused.len() {
                self.paused.remove(index);
                true
            } else {
                false
            }
        }
    }

    pub fn take(&mut self, index: usize) -> Option<Prompt> {
        if self.is_editing() {
            return None;
        }
        if index < self.messages.len() {
            self.messages.remove(index)
        } else {
            let index = index - self.messages.len();
            (index < self.paused.len()).then(|| self.paused.remove(index).take())
        }
    }

    pub fn pause(&mut self) {
        let count = self.messages.len();
        if count == 0 {
            return;
        }
        let mut paused: Vec<_> = self
            .messages
            .drain(..)
            .map(|prompt| {
                let mut editor = Editor::default();
                editor.set(prompt);
                editor
            })
            .collect();
        paused.append(&mut self.paused);
        self.paused = paused;
        // Combined index order is unchanged when queued entries become the
        // leading paused entries.
        if let Some(editing) = &self.editing {
            debug_assert!(editing.index < self.paused.len());
        }
    }

    pub fn snapshot(&self, editor: &Editor) -> (Vec<Prompt>, Vec<crate::sessions::Draft>) {
        let mut queued: Vec<_> = self.messages.iter().cloned().collect();
        let mut paused: Vec<_> = self.paused.iter().map(draft).collect();
        if let Some(editing) = &self.editing {
            if editing.index < queued.len() {
                queued[editing.index] = editor.prompt();
            } else {
                paused[editing.index - queued.len()] = draft(editor);
            }
        }
        (queued, paused)
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
