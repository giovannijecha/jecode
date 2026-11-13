mod cache;
mod chips;
mod layout;
#[cfg(test)]
mod reference_tests;
use super::terminal::Key;
use super::text;
use crate::attachments::{self, Attachment, MARKER, Prompt};
use chips::outside;
pub use layout::Layout;
use std::{cell::RefCell, ops::Range, rc::Rc};

const LIMIT: usize = 1024 * 1024;

#[derive(Default, Debug)]
pub struct Editor {
    pub text: String,
    pub cursor: usize,
    /// One per marker in `text`, in order. Only `attach` adds markers.
    pub attachments: Vec<Attachment>,
    goal: Option<usize>,
    /// In the chat composer, the start of the reference being typed at the caret. It
    /// stays plain, editable text until a space or any other action ends it.
    typing: Option<usize>,
    layout: RefCell<cache::Cache>,
}

impl Clone for Editor {
    fn clone(&self) -> Self {
        Self {
            text: self.text.clone(),
            cursor: self.cursor,
            attachments: self.attachments.clone(),
            goal: self.goal,
            typing: self.typing,
            layout: RefCell::default(),
        }
    }
}

impl PartialEq for Editor {
    fn eq(&self, other: &Self) -> bool {
        self.text == other.text
            && self.cursor == other.cursor
            && self.attachments == other.attachments
            && self.goal == other.goal
            && self.typing == other.typing
    }
}

impl Eq for Editor {}

impl Editor {
    pub fn insert(&mut self, text: &str) -> bool {
        self.typing = None;
        self.put(&input(text))
    }

    fn put(&mut self, text: &str) -> bool {
        if text.len() > LIMIT.saturating_sub(self.text.len()) {
            return false;
        }
        self.text.insert_str(self.cursor, text);
        self.cursor += text.len();
        self.goal = None;
        true
    }

    /// Inserts an attachment element at the cursor.
    pub fn attach(&mut self, attachment: Attachment) {
        self.typing = None;
        let index = self.markers(self.cursor);
        self.attachments.insert(index, attachment);
        self.text.insert(self.cursor, MARKER);
        self.cursor += MARKER.len_utf8();
        self.goal = None;
    }

    /// Replaces the byte range `start..end` with `text`, together with any attachments
    /// inside the range, leaving the caret after the new text.
    pub fn splice(&mut self, start: usize, end: usize, text: &str) -> bool {
        let text = attachments::strip(text);
        if text.len() > LIMIT.saturating_sub(self.text.len() - (end - start)) {
            return false;
        }
        self.remove(start, end);
        self.text.insert_str(start, &text);
        self.cursor = start + text.len();
        self.goal = None;
        self.typing = None;
        true
    }

    pub fn replace(&mut self, text: String) {
        self.set(Prompt::plain(text));
    }

    pub fn set(&mut self, prompt: Prompt) {
        // Recovery may combine several bounded messages. Never truncate recovered text.
        self.cursor = prompt.text.len();
        self.text = prompt.text;
        self.attachments = prompt.attachments;
        self.goal = None;
        self.typing = None;
    }

    pub fn prompt(&self) -> Prompt {
        Prompt::new(self.text.clone(), self.attachments.clone())
    }

    pub fn take(&mut self) -> Prompt {
        self.cursor = 0;
        self.goal = None;
        self.typing = None;
        Prompt::new(
            std::mem::take(&mut self.text),
            std::mem::take(&mut self.attachments),
        )
    }

    /// Text with numbered labels in place of markers.
    pub fn display(&self) -> String {
        attachments::expand(&self.text, &self.attachments)
    }

    fn markers(&self, end: usize) -> usize {
        self.text[..end].chars().filter(|&c| c == MARKER).count()
    }

    /// Removes a byte range together with the attachments it contains.
    fn remove(&mut self, start: usize, end: usize) {
        let first = self.markers(start);
        let count = self.text[start..end]
            .chars()
            .filter(|&c| c == MARKER)
            .count();
        self.attachments.drain(first..first + count);
        self.text.drain(start..end);
    }

    /// Rows for `columns` cells. The composer passes `references` to draw `$skill`
    /// and `@path` as chips, like attachments; other editors keep them as plain text.
    pub fn layout(&self, columns: usize, references: bool) -> Rc<Layout> {
        let labels = self
            .attachments
            .iter()
            .enumerate()
            .map(|(index, attachment)| attachment.chip(index + 1))
            .collect::<Vec<_>>();
        self.layout.borrow_mut().get(
            &self.text,
            self.cursor,
            columns,
            labels,
            references.then_some(self.typing),
        )
    }

    pub fn viewport(&self, columns: usize, secret: bool) -> (String, usize) {
        let columns = columns.max(1);
        let display = if secret {
            "•".repeat(self.text.chars().count())
        } else {
            text::clean(&self.display()).replace('\n', " ")
        };
        let before = if secret {
            self.text[..self.cursor].chars().count()
        } else {
            let prefix = attachments::expand(&self.text[..self.cursor], &self.attachments);
            text::cells(&text::clean(&prefix).replace('\n', " "))
        };
        let wanted = before.saturating_sub(columns - 1);
        let mut skipped = 0;
        let mut start = 0;
        for glyph in text::glyphs(&display) {
            if skipped >= wanted {
                break;
            }
            if skipped + glyph.cells > before {
                break;
            }
            skipped += glyph.cells;
            start += glyph.text.len();
        }
        (
            text::clip(&display[start..], columns),
            before.saturating_sub(skipped).min(columns - 1),
        )
    }

    pub fn vertical(&mut self, up: bool, columns: usize, references: bool) -> bool {
        if references {
            // Leaving the row ends a typed reference, which becomes a chip.
            self.typing = None;
            self.cursor = outside(&self.chips(), self.cursor, false);
        }
        let layout = self.layout(columns, references);
        let (row, column) = layout.cursor;
        let target = if up {
            row.checked_sub(1)
        } else {
            (row + 1 < layout.rows.len()).then_some(row + 1)
        };
        let Some(target) = target else {
            return false;
        };
        let column = *self.goal.get_or_insert(column);
        self.cursor = layout.byte_at(target, column);
        true
    }

    /// Ordinary editing, where every character is its own element.
    pub fn key(&mut self, key: Key) -> bool {
        self.typing = None;
        self.edit(key, &[])
    }

    /// Edits with `chips` as single elements: the caret moves across each whole and
    /// deleting any part of one removes all of it.
    fn edit(&mut self, key: Key, chips: &[Range<usize>]) -> bool {
        self.goal = None;
        let ctrl = key.ctrl();
        match key.code {
            13 if key.shift() || key.alt() => return self.put("\n"),
            74 if ctrl => return self.put("\n"),
            8 | 87 if key.code == 8 || ctrl => {
                let start = if ctrl {
                    self.word_left()
                } else {
                    self.previous()
                };
                self.erase(chips, start, self.cursor, true);
            }
            46 => {
                let end = if ctrl { self.word_right() } else { self.next() };
                self.erase(chips, self.cursor, end, false);
            }
            37 => {
                let target = if ctrl {
                    self.word_left()
                } else {
                    self.previous()
                };
                self.cursor = outside(chips, target, false);
            }
            39 => {
                let target = if ctrl { self.word_right() } else { self.next() };
                self.cursor = outside(chips, target, true);
            }
            36 => self.cursor = if ctrl { 0 } else { self.line_start() },
            35 => {
                self.cursor = if ctrl {
                    self.text.len()
                } else {
                    self.line_end()
                }
            }
            65 if ctrl => self.cursor = self.line_start(),
            69 if ctrl => self.cursor = self.line_end(),
            85 if ctrl => self.erase(chips, self.line_start(), self.cursor, true),
            _ => {}
        }
        true
    }

    fn line_start(&self) -> usize {
        self.text[..self.cursor]
            .rfind('\n')
            .map_or(0, |index| index + 1)
    }
    fn line_end(&self) -> usize {
        self.text[self.cursor..]
            .find('\n')
            .map_or(self.text.len(), |index| self.cursor + index)
    }
    fn previous(&self) -> usize {
        let anchor = self.local_anchor();
        let mut byte = anchor;
        for glyph in text::glyphs(&self.text[anchor..]) {
            let end = byte + glyph.text.len();
            if end >= self.cursor {
                return byte;
            }
            byte = end;
        }
        anchor
    }
    fn next(&self) -> usize {
        let anchor = self.local_anchor();
        let mut byte = anchor;
        for glyph in text::glyphs(&self.text[anchor..]) {
            byte += glyph.text.len();
            if byte > self.cursor {
                return byte;
            }
        }
        byte
    }
    fn local_anchor(&self) -> usize {
        let mut chars = self.text[..self.cursor].char_indices().rev().peekable();
        let Some((mut start, mut character)) = chars.next() else {
            return 0;
        };
        while text::extend(character) || character == '\u{200d}' {
            let Some((index, previous)) = chars.next() else {
                return start;
            };
            start = index;
            character = previous;
        }
        if regional(character) {
            while chars
                .peek()
                .is_some_and(|(_, previous)| regional(*previous))
            {
                start = chars.next().unwrap().0;
            }
            return start;
        }
        while text::emoji_base(character)
            && chars.peek().is_some_and(|(_, next)| *next == '\u{200d}')
        {
            chars.next();
            let Some((mut index, mut previous)) = chars.next() else {
                break;
            };
            while text::extend(previous) {
                let Some((earlier, earlier_character)) = chars.next() else {
                    return start;
                };
                index = earlier;
                previous = earlier_character;
            }
            if !text::emoji_base(previous) {
                break;
            }
            start = index;
            character = previous;
        }
        start
    }

    fn word_left(&self) -> usize {
        let mut chars = self.text[..self.cursor].char_indices().rev().peekable();
        let mut start = self.cursor;
        while chars.peek().is_some_and(|(_, ch)| ch.is_whitespace()) {
            start = chars.next().unwrap().0;
        }
        let Some((_, first)) = chars.peek() else {
            return start;
        };
        let class = word(*first);
        while chars
            .peek()
            .is_some_and(|(_, ch)| !ch.is_whitespace() && word(*ch) == class)
        {
            start = chars.next().unwrap().0;
        }
        start
    }

    fn word_right(&self) -> usize {
        let mut chars = self.text[self.cursor..].char_indices().peekable();
        let mut end = self.cursor;
        while chars.peek().is_some_and(|(_, ch)| ch.is_whitespace()) {
            let (offset, ch) = chars.next().unwrap();
            end = self.cursor + offset + ch.len_utf8();
        }
        let Some((_, first)) = chars.peek() else {
            return end;
        };
        let class = word(*first);
        while chars
            .peek()
            .is_some_and(|(_, ch)| !ch.is_whitespace() && word(*ch) == class)
        {
            let (offset, ch) = chars.next().unwrap();
            end = self.cursor + offset + ch.len_utf8();
        }
        end
    }
}

/// Typed or pasted text with line breaks normalized. It never carries attachment
/// markers.
fn input(text: &str) -> String {
    attachments::strip(&text.replace("\r\n", "\n").replace('\r', "\n"))
}

fn word(ch: char) -> bool {
    ch.is_alphanumeric() || ch == '_'
}

fn regional(character: char) -> bool {
    matches!(character as u32, 0x1f1e6..=0x1f1ff)
}

#[cfg(test)]
mod tests;
