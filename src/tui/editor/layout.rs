use crate::attachments::MARKER;
use crate::tui::text;
use std::{borrow::Cow, ops::Range};

#[derive(Clone, Debug)]
pub struct Row {
    pub text: String,
    pub positions: Vec<(usize, usize)>,
    /// Byte ranges of `text` that show attachment or reference chips.
    pub labels: Vec<Range<usize>>,
}

impl Row {
    fn new(byte: usize) -> Self {
        Self {
            text: String::new(),
            positions: vec![(byte, 0)],
            labels: Vec::new(),
        }
    }

    fn push(&mut self, value: &str, chip: bool) {
        let start = self.text.len();
        self.text.push_str(value);
        if chip {
            self.labels.push(start..self.text.len());
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Layout {
    pub rows: Vec<Row>,
    pub cursor: (usize, usize),
}

impl Layout {
    /// `labels` replace the attachment markers of `source`, in order, and the
    /// `chips` byte ranges of `source` show as reference chips. Each label and chip
    /// is one unit: the cursor and wrapping never split it, and a label too wide
    /// for a row is shortened on screen only.
    pub fn new(
        source: &str,
        cursor: usize,
        columns: usize,
        labels: &[String],
        chips: &[Range<usize>],
    ) -> Self {
        let mut layout = Self::default();
        layout.reflow(source, 0, columns, labels, chips);
        layout.move_cursor(cursor);
        layout
    }

    pub(super) fn reflow(
        &mut self,
        source: &str,
        row: usize,
        columns: usize,
        labels: &[String],
        chips: &[Range<usize>],
    ) {
        let columns = columns.max(4);
        let mut byte = self.rows.get(row).map_or(0, |row| row.positions[0].0);
        self.rows.truncate(row);
        self.rows.push(Row::new(byte));
        let mut cells = 0;
        let mut chips = chips[chips.partition_point(|chip| chip.end <= byte)..]
            .iter()
            .peekable();
        let mut label = if labels.is_empty() {
            0
        } else {
            source[..byte].chars().filter(|&ch| ch == MARKER).count()
        };
        let mut glyphs = text::glyphs(&source[byte..]);
        loop {
            if let Some(chip) = chips.next_if(|chip| chip.start <= byte) {
                let name = text::ellipsize(&text::clean(&source[chip.clone()]), columns - 2);
                let display = format!(" {name} ");
                let size = text::cells(&display);
                if cells + size > columns {
                    self.rows.push(Row::new(byte));
                    cells = 0;
                }
                let row = self.rows.last_mut().unwrap();
                row.push(&display, true);
                cells += size;
                byte = chip.end;
                row.positions.push((byte, cells));
                glyphs = text::glyphs(&source[byte..]);
                continue;
            }
            let Some(glyph) = glyphs.next() else {
                break;
            };
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
            row.push(&display, marker);
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

    /// The byte whose caret position is nearest `column` on `row`, so a chip is
    /// entered by neither end.
    pub fn byte_at(&self, row: usize, column: usize) -> usize {
        let positions = &self.rows[row].positions;
        let after = positions.partition_point(|(_, cell)| *cell <= column);
        let before = positions[after.saturating_sub(1)];
        match positions.get(after) {
            Some(&(byte, cell)) if after > 0 && cell - column < column - before.1 => byte,
            _ => before.0,
        }
    }
}
