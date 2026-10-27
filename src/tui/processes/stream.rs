//! Bounded saved-stream windows and their display positions.
use super::{Stream, ansi};
use crate::{tools::processes::Page, tui::text};
use std::collections::VecDeque;

/// Loaded log text stays within this many saved bytes.
pub(super) const RETAINED: u64 = 128 * 1024;

impl Default for Stream {
    fn default() -> Self {
        Self {
            pages: VecDeque::new(),
            offset: 0,
            anchor: 0,
            follow: true,
            error: None,
        }
    }
}

impl Stream {
    pub fn text(&self) -> String {
        self.pages.iter().map(|page| page.text.as_str()).collect()
    }

    pub fn display_text(&self, terminal: bool) -> String {
        let text = self.text();
        if terminal {
            ansi::transcript(&text)
        } else {
            text
        }
    }

    pub(super) fn start(&self) -> u64 {
        self.pages.front().map_or(0, |page| page.start)
    }

    pub(super) fn end(&self) -> u64 {
        self.pages.back().map_or(0, |page| page.end)
    }

    pub fn length(&self) -> u64 {
        self.pages.back().map_or(0, |page| page.length)
    }

    pub(super) fn display_chars(text: &str, terminal: bool) -> usize {
        let visible = if terminal {
            ansi::transcript(text)
        } else {
            text.to_owned()
        };
        text::clean(&visible).chars().count()
    }

    pub(super) fn select_row(&mut self, starts: &[usize], row: usize) {
        self.offset = row;
        self.anchor = starts.get(row).copied().unwrap_or(0);
    }

    /// Appends output after the window; returns the pages trimmed from its start.
    pub(super) fn append(&mut self, page: Page) -> Vec<Page> {
        if page.start != self.end() {
            self.pages.clear();
        }
        if page.end > page.start || self.pages.is_empty() {
            self.pages.push_back(page);
        } else if let Some(last) = self.pages.back_mut() {
            last.length = page.length;
        }
        let mut trimmed = Vec::new();
        while self.end() - self.start() > RETAINED && self.pages.len() > 1 {
            trimmed.extend(self.pages.pop_front());
        }
        trimmed
    }

    /// Prepends output before the window and trims its end to stay bounded.
    pub(super) fn prepend(&mut self, page: Page) {
        if page.end != self.start() {
            return;
        }
        self.pages.push_front(page);
        while self.end() - self.start() > RETAINED && self.pages.len() > 1 {
            self.pages.pop_back();
        }
    }
}
