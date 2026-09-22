//! Conservative display/edit units. This is not a complete Unicode table engine.
//! Boundaries are only introduced between printable ASCII characters. Non-ASCII
//! sequences and their adjacent bases stay together, possibly merging graphemes.
pub fn boundaries(text: &str) -> Vec<usize> {
    let mut result = vec![0];
    let mut previous = '\0';
    for (index, current) in text.char_indices() {
        if index != 0 && previous.is_ascii() && current.is_ascii() {
            result.push(index);
        }
        previous = current;
    }
    if !text.is_empty() {
        result.push(text.len());
    }
    result
}

pub fn safe(text: &str) -> String {
    text.chars()
        .map(|ch| match ch {
            '\n' | '\r' => '\n',
            '\t' => ' ',
            ch if ch.is_control()
                || matches!(ch, '\u{061c}' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}') =>
            {
                '?'
            }
            ch => ch,
        })
        .collect()
}

/// One cell for ASCII, Latin letters and our UI symbols; a conservative two-cell
/// fallback elsewhere. This is not a complete terminal Unicode width table.
pub fn width(text: &str) -> usize {
    text.chars()
        .map(|c| {
            if c.is_ascii()
                || matches!(c, '\u{c0}'..='\u{d6}' | '\u{d8}'..='\u{f6}' | '\u{f8}'..='\u{24f}' | '─' | '›' | '…' | '·' | '✓' | '\u{2800}'..='\u{28ff}')
            {
                1
            } else {
                2
            }
        })
        .sum()
}

pub fn wrap(text: &str, columns: usize) -> Vec<String> {
    let columns = columns.max(1);
    let clean = safe(text);
    let mut rows = Vec::new();
    for line in clean.split('\n') {
        let mut row = String::new();
        let mut used = 0;
        for pair in boundaries(line).windows(2) {
            let unit = &line[pair[0]..pair[1]];
            let size = width(unit);
            if used + size > columns && !row.is_empty() {
                let seam = row.rfind(' ').map(|index| index + 1).filter(|index| {
                    !unit.starts_with(' ')
                        && !row[..*index].trim().is_empty()
                        && boundaries(&row).contains(index)
                        && width(&row[*index..]) + size <= columns
                });
                if let Some(index) = seam {
                    let tail = row.split_off(index);
                    rows.push(std::mem::replace(&mut row, tail));
                    used = width(&row);
                } else {
                    rows.push(std::mem::take(&mut row));
                    used = 0;
                }
            }
            if size > columns {
                // Never split an unknown Unicode sequence to force it into a row.
                row.push_str(&".".repeat(columns.min(3)));
                used += columns.min(3);
            } else {
                row.push_str(unit);
                used += size;
            }
        }
        rows.push(row);
    }
    rows
}

#[derive(Default)]
pub struct Editor {
    pub text: String,
    pub cursor: usize,
}
impl Editor {
    pub fn insert(&mut self, text: &str) {
        let text = safe(&text.replace("\r\n", "\n")).replace('\n', " ");
        if self.text.len() + text.len() > 8192 {
            return;
        }
        self.text.insert_str(self.cursor, &text);
        self.cursor += text.len();
        // A new combining/ZWJ sequence may have swallowed the old boundary.
        self.cursor = boundaries(&self.text)
            .into_iter()
            .find(|b| *b >= self.cursor)
            .unwrap_or(self.text.len());
    }
    pub fn left(&mut self) {
        self.cursor = boundaries(&self.text)
            .into_iter()
            .rev()
            .find(|b| *b < self.cursor)
            .unwrap_or(0);
    }
    pub fn right(&mut self) {
        self.cursor = boundaries(&self.text)
            .into_iter()
            .find(|b| *b > self.cursor)
            .unwrap_or(self.text.len());
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
    pub fn take(&mut self) -> String {
        self.cursor = 0;
        std::mem::take(&mut self.text)
    }
}
