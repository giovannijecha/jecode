use crate::attachments::MARKER;
use crate::tui::text;

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

pub struct Layout {
    pub rows: Vec<Row>,
    pub cursor: (usize, usize),
}

impl Layout {
    /// `labels` replace the attachment markers of `source`, in order. A label
    /// is one unit: the cursor and wrapping never split it.
    pub fn new(source: &str, cursor: usize, columns: usize, labels: &[String]) -> Self {
        let columns = columns.max(4);
        let mut rows = vec![Row::new(0)];
        let mut cells = 0;
        let mut byte = 0;
        let mut label = 0;
        for glyph in text::glyphs(source) {
            if glyph.text == "\n" {
                rows.push(Row::new(byte + 1));
                cells = 0;
                byte += 1;
                continue;
            }
            let marker = glyph.text.starts_with(MARKER);
            let display = if marker {
                label += 1;
                let name = labels.get(label - 1).map_or("[Attachment]", String::as_str);
                text::ellipsize(&text::clean(name).replace('\n', " "), columns)
            } else {
                text::clean(glyph.text)
            };
            let size = text::cells(&display);
            if cells + size > columns {
                rows.push(Row::new(byte));
                cells = 0;
            }
            let row = rows.last_mut().unwrap();
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
            rows.push(Row::new(source.len()));
        }
        let location = rows
            .iter()
            .enumerate()
            .rev()
            .find_map(|(index, row)| {
                row.positions
                    .iter()
                    .rev()
                    .find(|(byte, _)| *byte <= cursor)
                    .map(|(_, column)| (index, *column))
            })
            .unwrap_or((0, 0));
        Self {
            rows,
            cursor: location,
        }
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
