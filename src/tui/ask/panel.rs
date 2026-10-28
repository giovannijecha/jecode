//! Answers in progress, one entry per question. Rows `0..options` are the
//! options; row `options` is `Other`, the user's own words, which is also
//! the only row of a question without options. The cursor row is never the
//! answer: only Enter, Space or typing selects.

use crate::ask::{Answer, AnswerDraft, Draft, Question, Request, TEXT_LIMIT};
use crate::tui::{editor::Editor, terminal::Key};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Entry {
    /// Selected options, in option order.
    pub picked: Vec<usize>,
    /// `Other` is selected: the typed words are part of the answer.
    pub other: bool,
    pub editor: Editor,
    /// The cursor row.
    pub row: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Panel {
    /// The question in front.
    pub step: usize,
    pub entries: Vec<Entry>,
    /// The block's first visible row once the user scrolls it; otherwise
    /// the block follows the cursor.
    pub top: Option<usize>,
}

impl Entry {
    fn restore(question: &Question, saved: &AnswerDraft) -> Self {
        let mut picked: Vec<usize> = question
            .options
            .iter()
            .enumerate()
            .filter(|(_, option)| saved.choices.contains(&option.id))
            .map(|(index, _)| index)
            .collect();
        if !question.is_multiple() {
            picked.truncate(1);
        }
        let mut editor = Editor::default();
        editor.replace(saved.text.clone());
        editor.cursor = saved.caret.min(editor.text.len());
        while !editor.text.is_char_boundary(editor.cursor) {
            editor.cursor -= 1;
        }
        let mut entry = Self {
            picked,
            other: saved.other,
            editor,
            row: saved.row.min(question.options.len()),
        };
        if entry.editor.text.trim().is_empty() {
            entry.other = false;
        } else if entry.other && !question.is_multiple() {
            entry.picked.clear();
        }
        entry
    }

    /// The answer this entry gives, if any.
    pub fn answer(&self, question: &Question) -> Option<Answer> {
        let typed = !self.editor.text.trim().is_empty();
        let own = typed && (self.other || question.options.is_empty());
        let choices: Vec<String> = if own && !question.is_multiple() {
            Vec::new()
        } else {
            self.picked
                .iter()
                .map(|&index| question.options[index].id.clone())
                .collect()
        };
        (own || !choices.is_empty()).then(|| Answer {
            id: question.id.clone(),
            choices,
            text: if own {
                self.editor.text.clone()
            } else {
                String::new()
            },
        })
    }

    /// The cursor is on the typed words.
    pub fn in_text(&self, question: &Question) -> bool {
        self.row == question.options.len()
    }

    /// Typed words become the selected answer; an emptied text is not.
    fn typed(&mut self, question: &Question) {
        if self.editor.text.trim().is_empty() {
            self.other = false;
        } else {
            self.other = true;
            if !question.is_multiple() {
                self.picked.clear();
            }
        }
    }
}

impl Panel {
    /// Nothing is selected until the user chooses; a saved draft for the
    /// same call restores the answers in progress.
    pub fn new(request: &Request, saved: Option<&Draft>) -> Self {
        let saved = saved.filter(|draft| {
            draft.id == request.id && draft.answers.len() == request.questions.len()
        });
        let entries = request
            .questions
            .iter()
            .enumerate()
            .map(|(index, question)| {
                saved.map_or_else(Entry::default, |draft| {
                    Entry::restore(question, &draft.answers[index])
                })
            })
            .collect();
        Self {
            step: saved.map_or(0, |draft| {
                draft.step.min(request.questions.len().saturating_sub(1))
            }),
            entries,
            top: None,
        }
    }

    pub fn draft(&self, request: &Request) -> Draft {
        Draft {
            id: request.id.clone(),
            step: self.step,
            answers: request
                .questions
                .iter()
                .zip(&self.entries)
                .map(|(question, entry)| AnswerDraft {
                    choices: entry
                        .picked
                        .iter()
                        .map(|&index| question.options[index].id.clone())
                        .collect(),
                    other: entry.other,
                    text: entry.editor.text.clone(),
                    caret: entry.editor.cursor,
                    row: entry.row,
                })
                .collect(),
        }
    }

    pub fn entry(&self) -> &Entry {
        &self.entries[self.step]
    }

    /// Every answer in order, or the first question still unanswered.
    pub fn answers(&self, request: &Request) -> Result<Vec<Answer>, usize> {
        request
            .questions
            .iter()
            .zip(&self.entries)
            .enumerate()
            .map(|(index, (question, entry))| entry.answer(question).ok_or(index))
            .collect()
    }

    /// Moves to the next or the previous question.
    pub fn switch(&mut self, request: &Request, forward: bool) -> bool {
        let next = if forward {
            (self.step + 1).min(request.questions.len() - 1)
        } else {
            self.step.saturating_sub(1)
        };
        let moved = next != self.step;
        self.step = next;
        self.top = None;
        moved
    }

    /// Moves the cursor between answer rows; nothing is selected.
    pub fn move_row(&mut self, question: &Question, up: bool) -> bool {
        let entry = &mut self.entries[self.step];
        let row = if up {
            entry.row.saturating_sub(1)
        } else {
            (entry.row + 1).min(question.options.len())
        };
        let moved = row != entry.row;
        entry.row = row;
        if moved && entry.in_text(question) {
            entry.editor.cursor = entry.editor.text.len();
        }
        self.top = None;
        moved
    }

    /// Space on an option: a single choice selects it, a multiple choice
    /// toggles it. Elsewhere Space is typed.
    pub fn select(&mut self, question: &Question) -> bool {
        let entry = &mut self.entries[self.step];
        if entry.in_text(question) {
            return false;
        }
        if question.is_multiple() {
            match entry.picked.binary_search(&entry.row) {
                Ok(index) => {
                    entry.picked.remove(index);
                }
                Err(index) => entry.picked.insert(index, entry.row),
            }
        } else {
            entry.picked = vec![entry.row];
            entry.other = false;
        }
        self.top = None;
        true
    }

    /// Enter on a single choice: the cursor row becomes the answer. False
    /// when the cursor is on empty own words.
    pub fn choose(&mut self, question: &Question) -> bool {
        let entry = &mut self.entries[self.step];
        if entry.in_text(question) {
            entry.typed(question);
            return entry.other;
        }
        entry.picked = vec![entry.row];
        entry.other = false;
        true
    }

    /// Typing goes to `Other` and selects it. False when the answer would
    /// exceed its limit.
    pub fn type_text(&mut self, question: &Question, text: &str) -> bool {
        let entry = &mut self.entries[self.step];
        if !entry.in_text(question) {
            entry.row = question.options.len();
            entry.editor.cursor = entry.editor.text.len();
        }
        self.top = None;
        if text.len() > TEXT_LIMIT.saturating_sub(entry.editor.text.len())
            || !entry.editor.insert(text)
        {
            return false;
        }
        entry.typed(question);
        true
    }

    /// An editing key on the typed words. False when a new line would
    /// exceed the limit.
    pub fn edit(&mut self, question: &Question, key: Key) -> bool {
        let entry = &mut self.entries[self.step];
        let newline = key.code == 13 && (key.shift() || key.alt()) || key.ctrl() && key.code == 74;
        if newline && entry.editor.text.len() >= TEXT_LIMIT {
            return false;
        }
        entry.editor.key(key);
        entry.typed(question);
        self.top = None;
        true
    }
}
