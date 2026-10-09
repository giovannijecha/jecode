use crate::tui::text;

pub struct Row {
    pub text: String,
    pub positions: Vec<(usize, usize)>,
}

pub struct Layout {
    pub rows: Vec<Row>,
    pub cursor: (usize, usize),
}

impl Layout {
    pub fn new(source: &str, cursor: usize, columns: usize) -> Self {
        let columns = columns.max(4);
        let mut rows = vec![Row {
            text: String::new(),
            positions: vec![(0, 0)],
        }];
        let mut cells = 0;
        let mut byte = 0;
        for glyph in text::glyphs(source) {
            if glyph.text == "\n" {
                rows.push(Row {
                    text: String::new(),
                    positions: vec![(byte + 1, 0)],
                });
                cells = 0;
                byte += 1;
                continue;
            }
            let display = text::clean(glyph.text);
            let size = text::cells(&display);
            if cells + size > columns {
                rows.push(Row {
                    text: String::new(),
                    positions: vec![(byte, 0)],
                });
                cells = 0;
            }
            let row = rows.last_mut().unwrap();
            row.text.push_str(&display);
            cells += size;
            byte += glyph.text.len();
            row.positions.push((byte, cells));
        }
        if cells >= columns {
            rows.push(Row {
                text: String::new(),
                positions: vec![(source.len(), 0)],
            });
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
