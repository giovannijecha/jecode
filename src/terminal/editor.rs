//! Draft editing and its pure visual-line map. Byte offsets always land on safe units.
use super::editor_visual::Visual;

pub const MAX_INPUT_BYTES: usize = crate::session::MAX_PROMPT_BYTES;

// Editing can use finer stops than transcript wrapping. This covers common
// extended clusters, but is not a complete Unicode grapheme-break database.
pub(super) fn boundaries(text: &str) -> Vec<usize> {
    let mut result = vec![0];
    let mut previous = '\0';
    let mut regional_run = 0;
    for (index, current) in text.char_indices() {
        let joined_regional_pair = regional(previous) && regional(current) && regional_run % 2 == 1;
        let joined = current == '\u{200d}'
            || previous == '\u{200d}'
            || extending(current)
            || prepend(previous)
            || virama(previous)
            || joined_regional_pair
            || conjoining_hangul(previous, current);
        let previous_base = previous.is_ascii()
            || previous.is_alphanumeric()
            || pictograph(previous)
            || previous.is_whitespace()
            || matches!(previous, '\u{0300}'..='\u{036f}');
        let current_base = current.is_ascii()
            || current.is_alphanumeric()
            || pictograph(current)
            || current.is_whitespace();
        if index != 0
            && (matches!(previous, '\n' | '\t')
                || matches!(current, '\n' | '\t')
                || !joined && previous_base && current_base)
        {
            result.push(index);
        }
        regional_run = if regional(current) {
            regional_run + 1
        } else {
            0
        };
        previous = current;
    }
    if !text.is_empty() {
        result.push(text.len());
    }
    result
}
fn regional(ch: char) -> bool {
    ('\u{1f1e6}'..='\u{1f1ff}').contains(&ch)
}
#[derive(Clone, Copy)]
enum Hangul {
    L,
    V,
    T,
    Lv,
    Lvt,
    Other,
}
fn hangul(ch: char) -> Hangul {
    match ch {
        '\u{1100}'..='\u{115f}' | '\u{a960}'..='\u{a97c}' => Hangul::L,
        '\u{1160}'..='\u{11a7}' | '\u{d7b0}'..='\u{d7c6}' => Hangul::V,
        '\u{11a8}'..='\u{11ff}' | '\u{d7cb}'..='\u{d7fb}' => Hangul::T,
        '\u{ac00}'..='\u{d7a3}' if (ch as u32 - 0xac00).is_multiple_of(28) => Hangul::Lv,
        '\u{ac00}'..='\u{d7a3}' => Hangul::Lvt,
        _ => Hangul::Other,
    }
}
// Conjoining Jamo and precomposed syllables follow the three Hangul
// continuation rules; every editor and visual stop uses this same boundary map.
fn conjoining_hangul(previous: char, current: char) -> bool {
    use Hangul::{L, Lv, Lvt, T, V};
    matches!(
        (hangul(previous), hangul(current)),
        (L, L | V | Lv | Lvt) | (Lv | V, V | T) | (Lvt | T, T)
    )
}
fn pictograph(ch: char) -> bool {
    matches!(ch, '\u{2600}'..='\u{27bf}' | '\u{1f000}'..='\u{1faff}')
}
fn prepend(ch: char) -> bool {
    matches!(
        ch,
        '\u{0600}'..='\u{0605}' | '\u{06dd}' | '\u{070f}' | '\u{08e2}'
    )
}
fn virama(ch: char) -> bool {
    matches!(
        ch,
        '\u{094d}'
            | '\u{09cd}'
            | '\u{0a4d}'
            | '\u{0acd}'
            | '\u{0b4d}'
            | '\u{0bcd}'
            | '\u{0c4d}'
            | '\u{0ccd}'
            | '\u{0d4d}'
    )
}
fn extending(ch: char) -> bool {
    matches!(ch,
        '\u{0300}'..='\u{036f}' | '\u{0483}'..='\u{0489}' |
        '\u{0591}'..='\u{05bd}' | '\u{05bf}' | '\u{05c1}'..='\u{05c5}' |
        '\u{0610}'..='\u{061a}' | '\u{064b}'..='\u{065f}' |
        '\u{0670}' | '\u{06d6}'..='\u{06ed}' |
        '\u{0900}'..='\u{0903}' | '\u{093a}'..='\u{094f}' |
        '\u{1ab0}'..='\u{1aff}' | '\u{1dc0}'..='\u{1dff}' |
        '\u{20d0}'..='\u{20ff}' | '\u{fe00}'..='\u{fe0f}' |
        '\u{fe20}'..='\u{fe2f}' | '\u{1f3fb}'..='\u{1f3ff}' |
        '\u{e0100}'..='\u{e01ef}')
}

#[derive(Clone)]
pub struct Editor {
    pub text: String,
    pub cursor: usize,
    columns: usize,
    preferred_column: Option<usize>,
}
impl Default for Editor {
    fn default() -> Self {
        Self {
            text: String::new(),
            cursor: 0,
            columns: 77,
            preferred_column: None,
        }
    }
}
impl Editor {
    pub fn set_columns(&mut self, columns: usize) {
        self.columns = columns.max(1);
    }
    pub fn columns(&self) -> usize {
        self.columns
    }
    /// Reject the entire insertion when it would exceed the canonical prompt limit.
    pub fn insert(&mut self, input: &str) -> bool {
        let normalized = input.replace("\r\n", "\n").replace('\r', "\n");
        let clean: String = normalized
            .chars()
            .map(|ch| match ch {
                '\n' | '\t' => ch,
                ch if ch.is_control()
                    || matches!(ch, '\u{061c}' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}') =>
                {
                    '?'
                }
                ch => ch,
            })
            .collect();
        if self.text.len().saturating_add(clean.len()) > MAX_INPUT_BYTES {
            return false;
        }
        self.text.insert_str(self.cursor, &clean);
        self.cursor += clean.len();
        // A combining/ZWJ sequence can absorb the insertion boundary.
        self.cursor = boundaries(&self.text)
            .into_iter()
            .find(|b| *b >= self.cursor)
            .unwrap_or(self.text.len());
        self.preferred_column = None;
        true
    }
    pub fn replace(&mut self, value: &str) {
        self.text = value.to_owned();
        self.cursor = self.text.len();
        self.preferred_column = None;
    }
    pub fn left(&mut self) {
        self.cursor = boundaries(&self.text)
            .into_iter()
            .rev()
            .find(|b| *b < self.cursor)
            .unwrap_or(0);
        self.preferred_column = None;
    }
    pub fn right(&mut self) {
        self.cursor = boundaries(&self.text)
            .into_iter()
            .find(|b| *b > self.cursor)
            .unwrap_or(self.text.len());
        self.preferred_column = None;
    }
    pub fn home(&mut self) {
        self.cursor = self.text[..self.cursor].rfind('\n').map_or(0, |n| n + 1);
        self.preferred_column = None;
    }
    pub fn end(&mut self) {
        self.cursor = self.text[self.cursor..]
            .find('\n')
            .map_or(self.text.len(), |n| self.cursor + n);
        self.preferred_column = None;
    }
    pub fn draft_start(&mut self) {
        self.cursor = 0;
        self.preferred_column = None;
    }
    pub fn draft_end(&mut self) {
        self.cursor = self.text.len();
        self.preferred_column = None;
    }
    pub fn word_left(&mut self) {
        self.cursor = self.previous_word();
        self.preferred_column = None;
    }
    pub fn word_right(&mut self) {
        self.cursor = self.next_word();
        self.preferred_column = None;
    }
    pub fn backspace(&mut self) {
        let end = self.cursor;
        self.left();
        self.text.drain(self.cursor..end);
    }
    pub fn delete(&mut self) {
        let start = self.cursor;
        self.right();
        self.text.drain(start..self.cursor);
        self.cursor = start;
    }
    pub fn word_backspace(&mut self) {
        let start = self.previous_word();
        self.text.drain(start..self.cursor);
        self.cursor = start;
        self.preferred_column = None;
    }
    pub fn word_delete(&mut self) {
        let end = self.next_word();
        self.text.drain(self.cursor..end);
        self.preferred_column = None;
    }
    fn previous_word(&self) -> usize {
        let stops = boundaries(&self.text);
        let mut index = stops.binary_search(&self.cursor).unwrap_or(0);
        while index > 0 && category(&self.text[stops[index - 1]..stops[index]]) == 0 {
            index -= 1;
        }
        if index > 0 {
            let kind = category(&self.text[stops[index - 1]..stops[index]]);
            while index > 0 && category(&self.text[stops[index - 1]..stops[index]]) == kind {
                index -= 1;
            }
        }
        stops[index]
    }
    fn next_word(&self) -> usize {
        let stops = boundaries(&self.text);
        let mut index = stops.binary_search(&self.cursor).unwrap_or(0);
        let last = stops.len() - 1;
        if index < last {
            let kind = category(&self.text[stops[index]..stops[index + 1]]);
            while index < last && category(&self.text[stops[index]..stops[index + 1]]) == kind {
                index += 1;
            }
            while index < last && category(&self.text[stops[index]..stops[index + 1]]) == 0 {
                index += 1;
            }
        }
        stops[index]
    }
    pub fn vertical(&mut self, down: bool) -> bool {
        let layout = Visual::new(&self.text, self.columns);
        let current = layout.stop(self.cursor);
        let target = if down {
            current
                .row
                .checked_add(1)
                .filter(|row| *row < layout.rows.len())
        } else {
            current.row.checked_sub(1)
        };
        let Some(target) = target else {
            return false;
        };
        let column = *self.preferred_column.get_or_insert(current.column);
        self.cursor = layout
            .stops
            .iter()
            .filter(|stop| stop.row == target)
            .min_by_key(|stop| (stop.column.abs_diff(column), stop.column > column))
            .map_or(self.cursor, |stop| stop.index);
        true
    }
    pub fn has_visual_lines(&self) -> bool {
        Visual::new(&self.text, self.columns).rows.len() > 1
    }
    pub fn take(&mut self) -> String {
        self.cursor = 0;
        self.preferred_column = None;
        std::mem::take(&mut self.text)
    }
}

fn category(unit: &str) -> u8 {
    let ch = unit.chars().next().unwrap_or(' ');
    if ch.is_whitespace() {
        0
    } else if ch.is_alphanumeric() || ch == '_' {
        1
    } else {
        2
    }
}
