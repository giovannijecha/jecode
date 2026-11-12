use crate::attachments::MARKER;
use crate::tui::{references::syntax, text};
use std::borrow::Cow;

#[derive(Clone, Debug)]
pub struct Row {
    pub text: String,
    pub positions: Vec<(usize, usize)>,
    /// Byte ranges of `text` that show attachment or reference chips.
    pub labels: Vec<std::ops::Range<usize>>,
}

impl Row {
    fn new(byte: usize) -> Self {
        Self {
            text: String::new(),
            positions: vec![(byte, 0)],
            labels: Vec::new(),
        }
    }

    fn push(&mut self, value: &str, chip: bool, join: bool) {
        let start = self.text.len();
        self.text.push_str(value);
        if chip {
            if join && let Some(last) = self.labels.last_mut().filter(|last| last.end == start) {
                last.end = self.text.len();
            } else {
                self.labels.push(start..self.text.len());
            }
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Layout {
    pub rows: Vec<Row>,
    pub cursor: (usize, usize),
}

impl Layout {
    /// `labels` replace the attachment markers of `source`, in order. A label
    /// is one unit: the cursor and wrapping never split it. With `references`,
    /// `$skill` and `@path` references are drawn as chips too.
    pub fn new(
        source: &str,
        cursor: usize,
        columns: usize,
        labels: &[String],
        references: bool,
    ) -> Self {
        let mut layout = Self::default();
        layout.reflow(source, 0, columns, labels, references);
        layout.move_cursor(cursor);
        layout
    }

    pub(super) fn reflow(
        &mut self,
        source: &str,
        row: usize,
        columns: usize,
        labels: &[String],
        references: bool,
    ) {
        let columns = columns.max(4);
        let mut byte = self.rows.get(row).map_or(0, |row| row.positions[0].0);
        self.rows.truncate(row);
        self.rows.push(Row::new(byte));
        let mut cells = 0;
        let tokens = if references && !source.starts_with('/') {
            syntax::tokens(source)
        } else {
            Vec::new()
        };
        let from = byte;
        let mut references = tokens
            .iter()
            .filter(|token| !token.value.is_empty() && token.end > from)
            .peekable();
        let mut label = if labels.is_empty() {
            0
        } else {
            source[..byte].chars().filter(|&ch| ch == MARKER).count()
        };
        for glyph in text::glyphs(&source[byte..]) {
            if glyph.text == "\n" {
                self.rows.push(Row::new(byte + 1));
                cells = 0;
                byte += 1;
                continue;
            }
            while references.peek().is_some_and(|token| token.end <= byte) {
                references.next();
            }
            let reference = references.peek().filter(|token| token.start <= byte);
            let leading = reference.is_some_and(|token| token.start == byte);
            let trailing = reference.is_some_and(|token| token.end == byte + glyph.text.len());
            // Move a reference that fits onto the next row as one card. Longer
            // references keep all their text and their surface across rows.
            if leading {
                let token = reference.unwrap();
                let size = text::cells(&text::clean(&source[token.start..token.end])) + 2;
                if size <= columns && cells + size > columns {
                    self.rows.push(Row::new(byte));
                    cells = 0;
                }
            }
            let marker = glyph.text.starts_with(MARKER);
            let display = if marker {
                label += 1;
                let name = labels.get(label - 1).map_or("[Attachment]", String::as_str);
                Cow::Owned(text::ellipsize(
                    &text::clean(name).replace('\n', " "),
                    columns,
                ))
            } else if glyph.text.chars().any(char::is_control) {
                Cow::Owned(text::clean(glyph.text))
            } else {
                Cow::Borrowed(glyph.text)
            };
            let size = match &display {
                Cow::Borrowed(_) => glyph.cells,
                Cow::Owned(value) => text::cells(value),
            };
            if cells + size + usize::from(leading) + usize::from(trailing) > columns {
                self.rows.push(Row::new(byte));
                cells = 0;
            }
            let row = self.rows.last_mut().unwrap();
            if leading {
                row.push(" ", true, false);
                cells += 1;
                row.positions.push((byte, cells));
            }
            row.push(&display, marker || reference.is_some(), reference.is_some());
            cells += size;
            byte += glyph.text.len();
            if trailing {
                // The padding cell also maps to the reference's end.
                row.positions.push((byte, cells));
                row.push(" ", true, true);
                cells += 1;
            }
            row.positions.push((byte, cells));
        }
        if cells >= columns {
            self.rows.push(Row::new(source.len()));
        }
    }

    pub(super) fn move_cursor(&mut self, cursor: usize) {
        self.cursor = self.cursor_position(cursor);
    }

    pub(super) fn cursor_position(&self, cursor: usize) -> (usize, usize) {
        let row = self
            .rows
            .partition_point(|row| row.positions[0].0 <= cursor)
            .saturating_sub(1);
        let positions = &self.rows[row].positions;
        let position = positions
            .partition_point(|(byte, _)| *byte <= cursor)
            .saturating_sub(1);
        (row, positions[position].1)
    }

    pub fn byte_at(&self, row: usize, column: usize) -> usize {
        self.rows[row]
            .positions
            .iter()
            .rev()
            .find(|(_, cell)| *cell <= column)
            .or_else(|| self.rows[row].positions.first())
            .unwrap()
            .0
    }
}
