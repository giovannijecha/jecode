//! Reference chips in the chat composer. Each nonempty `$skill` or `@path` reference
//! drawn as a chip is one element, like an attachment: the caret moves across it whole,
//! deleting any part removes all of it, and text typed or pasted beside it never
//! changes its value. The saved and sent text keeps the reference syntax unchanged.
//!
//! A reference typed by hand stays plain, editable text while the caret remains in its
//! word, so it can be written and corrected; a space or any other action ends it.

use super::{Editor, Key, input};
use crate::tui::references::syntax::{self, boundary};
use std::ops::Range;

/// The chips of `text`: its nonempty references outside a slash command, except the
/// one being typed from `typing`. Editing and layout share this one definition.
pub(super) fn chips(text: &str, typing: Option<usize>) -> Vec<Range<usize>> {
    let mut chips = syntax::chips(text);
    chips.retain(|chip| Some(chip.start) != typing);
    chips
}

/// The chip strictly containing `byte`.
fn inside(chips: &[Range<usize>], byte: usize) -> Option<&Range<usize>> {
    let index = chips.partition_point(|chip| chip.end <= byte);
    chips.get(index).filter(|chip| chip.start < byte)
}

/// `byte`, moved out of a chip it falls inside: to its end going forward, else its start.
pub(super) fn outside(chips: &[Range<usize>], byte: usize, forward: bool) -> usize {
    inside(chips, byte).map_or(byte, |chip| if forward { chip.end } else { chip.start })
}

/// `start..end` widened to whole chips.
fn whole(chips: &[Range<usize>], start: usize, end: usize) -> (usize, usize) {
    (outside(chips, start, false), outside(chips, end, true))
}

impl Editor {
    pub(super) fn chips(&self) -> Vec<Range<usize>> {
        chips(&self.text, self.typing)
    }

    /// Text typed (`typed`) or pasted into the chat composer. Beside a chip, a space keeps
    /// the boundary when the text would otherwise join the chip or end its reference;
    /// the caret stays after the text itself.
    pub fn compose(&mut self, text: &str, typed: bool) -> bool {
        let text = input(text);
        let chips = self.chips();
        self.cursor = outside(&chips, self.cursor, false);
        let at = self.cursor;
        // A space goes only where no boundary is already. Without a separation that
        // keeps every chip, the text goes in as typed.
        let lead = !boundary(self.text[..at].chars().next_back());
        let trail = !boundary(self.text[at..].chars().next());
        let (before, after) = [("", ""), (" ", ""), ("", " "), (" ", " ")]
            .into_iter()
            .filter(|(before, after)| (before.is_empty() || lead) && (after.is_empty() || trail))
            .find(|(before, after)| {
                chips.is_empty() || {
                    let added = format!("{before}{text}{after}");
                    let mut trial = self.text.clone();
                    trial.insert_str(at, &added);
                    let shift = |chip: &Range<usize>| {
                        let shift = if chip.start >= at { added.len() } else { 0 };
                        chip.start + shift..chip.end + shift
                    };
                    survive(&trial, chips.iter().map(shift))
                }
            })
            .unwrap_or_default();
        if !self.put(&format!("{before}{text}{after}")) {
            return false;
        }
        self.cursor -= after.len();
        self.typing = self.typed(&chips, typed);
        true
    }

    /// An editing key in the chat composer. Backspace still corrects a reference being
    /// typed; every other key ends it first.
    pub fn compose_key(&mut self, key: Key) -> bool {
        let correcting = key.code == 8 && !key.ctrl() && self.typing.is_some();
        if !correcting {
            self.typing = None;
        }
        let chips = self.chips();
        self.cursor = outside(&chips, self.cursor, false);
        let result = self.edit(key, &chips);
        self.typing = if correcting {
            self.typed(&chips, true)
        } else {
            None
        };
        result
    }

    /// Removes `start..end`, widened to whole chips. A removal that would join a
    /// remaining chip to its neighbours or end its reference moves the caret across
    /// the range instead, `backward` or forward, so boundaries stay unambiguous.
    pub(super) fn erase(
        &mut self,
        chips: &[Range<usize>],
        start: usize,
        end: usize,
        backward: bool,
    ) {
        let (start, end) = whole(chips, start, end);
        if !chips.is_empty() && start < end {
            let mut trial = self.text.clone();
            trial.replace_range(start..end, "");
            let removed = end - start;
            let remaining = chips
                .iter()
                .filter(|chip| chip.end <= start || chip.start >= end)
                .map(|chip| {
                    let shift = if chip.start >= end { removed } else { 0 };
                    chip.start - shift..chip.end - shift
                });
            if !survive(&trial, remaining) {
                self.cursor = if backward { start } else { end };
                return;
            }
        }
        self.remove(start, end);
        self.cursor = start;
    }

    /// The reference being typed after an edit: it starts before the caret, the caret is
    /// inside it or, while `typed`, after it in the same word, and it was not a chip.
    fn typed(&self, chips: &[Range<usize>], typed: bool) -> Option<usize> {
        let token = syntax::tokens(&self.text)
            .into_iter()
            .take_while(|token| token.start < self.cursor)
            .last()?;
        let word = self.text.get(token.end..self.cursor).unwrap_or_default();
        let open = self.cursor < token.end || typed && !word.chars().any(|ch| boundary(Some(ch)));
        (open && !chips.iter().any(|chip| chip.start == token.start)).then_some(token.start)
    }
}

/// Whether `text` still holds every `expected` chip. A draft that became a slash
/// command keeps none: its arguments are plain text, as typed.
fn survive(text: &str, expected: impl IntoIterator<Item = Range<usize>>) -> bool {
    if text.starts_with('/') {
        return true;
    }
    let after = syntax::chips(text);
    expected.into_iter().all(|chip| {
        after
            .binary_search_by_key(&chip.start, |found| found.start)
            .is_ok_and(|index| after[index] == chip)
    })
}
