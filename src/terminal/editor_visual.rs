//! Pure composer wrapping and byte-to-cell stops shared by drawing and vertical movement.
use super::{editor::boundaries, text};

#[derive(Clone, Copy)]
pub struct Stop {
    pub index: usize,
    pub row: usize,
    pub column: usize,
    pub byte: usize,
    /// End of the unit starting here, before a later soft wrap can move its
    /// following insertion stop to the next row.
    pub display_end: usize,
}
pub struct Visual {
    pub rows: Vec<String>,
    pub stops: Vec<Stop>,
}
impl Visual {
    pub fn new(input: &str, columns: usize) -> Self {
        let columns = columns.max(1);
        let boundaries = boundaries(input);
        let mut rows = vec![String::new()];
        let mut stops = vec![
            Stop {
                index: 0,
                row: 0,
                column: 0,
                byte: 0,
                display_end: 0,
            };
            boundaries.len()
        ];
        let mut column = 0;
        for (i, pair) in boundaries.windows(2).enumerate() {
            let unit = &input[pair[0]..pair[1]];
            if unit == "\n" {
                stops[i] = Stop {
                    index: pair[0],
                    row: rows.len() - 1,
                    column,
                    byte: rows.last().unwrap().len(),
                    display_end: rows.last().unwrap().len(),
                };
                rows.push(String::new());
                column = 0;
                stops[i + 1] = Stop {
                    index: pair[1],
                    row: rows.len() - 1,
                    column,
                    byte: 0,
                    display_end: 0,
                };
                continue;
            }
            let mut shown = if unit == "\t" {
                " ".repeat((4 - column % 4).min(columns))
            } else {
                text::safe(unit)
            };
            let mut size = text::width(&shown);
            if column + size > columns && column != 0 {
                rows.push(String::new());
                column = 0;
                if unit == "\t" {
                    shown = " ".repeat(4.min(columns));
                    size = shown.len();
                }
            }
            if size > columns {
                shown = ".".repeat(columns.min(3));
                size = shown.len();
            }
            let start_byte = rows.last().unwrap().len();
            rows.last_mut().unwrap().push_str(&shown);
            stops[i] = Stop {
                index: pair[0],
                row: rows.len() - 1,
                column,
                byte: start_byte,
                display_end: start_byte + shown.len(),
            };
            column += size;
            stops[i + 1] = Stop {
                index: pair[1],
                row: rows.len() - 1,
                column,
                byte: rows.last().unwrap().len(),
                display_end: rows.last().unwrap().len(),
            };
        }
        Self { rows, stops }
    }
    pub fn stop(&self, index: usize) -> Stop {
        self.stops
            .iter()
            .find(|stop| stop.index == index)
            .copied()
            .unwrap_or(*self.stops.last().unwrap())
    }
}
