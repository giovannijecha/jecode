use crate::attachments::MARKER;
use crate::tui::text;
use std::borrow::Cow;

#[derive(Clone, Debug)]
pub struct Row {
    pub text: String,
    pub positions: Vec<(usize, usize)>,
    /// Byte ranges of `text` that show attachment labels.
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
}

#[derive(Clone, Debug, Default)]
pub struct Layout {
    pub rows: Vec<Row>,
    pub cursor: (usize, usize),
}

impl Layout {
    /// `labels` replace the attachment markers of `source`, in order. A label
    /// is one unit: the cursor and wrapping never split it.
    pub fn new(source: &str, cursor: usize, columns: usize, labels: &[String]) -> Self {
        let mut layout = Self::default();
        layout.reflow(source, 0, columns, labels);
        layout.move_cursor(cursor);
        layout
    }

    pub(super) fn reflow(&mut self, source: &str, row: usize, columns: usize, labels: &[String]) {
        let columns = columns.max(4);
        let mut byte = self.rows.get(row).map_or(0, |row| row.positions[0].0);
        self.rows.truncate(row);
        self.rows.push(Row::new(byte));
        let mut cells = 0;
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
            if cells + size > columns {
                self.rows.push(Row::new(byte));
                cells = 0;
            }
            let row = self.rows.last_mut().unwrap();
            if marker {
                row.labels
                    .push(row.text.len()..row.text.len() + display.len());
            }
            row.text.push_str(&display);
            cells += size;
            byte += glyph.text.len();
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
