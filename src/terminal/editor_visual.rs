//! One grapheme-safe source-to-display map for the editor and composer.
use super::{editor::boundaries, lab::text};
use std::ops::Range;

#[derive(Clone, Copy)]
pub struct Stop {
    pub index: usize,
    pub row: usize,
    pub column: usize,
    #[allow(dead_code)]
    pub byte: usize,
    #[allow(dead_code)]
    pub display_end: usize,
}
pub struct Visual {
    pub rows: Vec<String>,
    pub stops: Vec<Stop>,
}
impl Visual {
    pub fn new(input: &str, columns: usize) -> Self {
        let columns = columns.max(1);
        let source = boundaries(input);
        let mut clean = String::with_capacity(input.len());
        let mut display_offsets = vec![0];
        for pair in source.windows(2) {
            clean.push_str(&text::safe(&input[pair[0]..pair[1]]));
            display_offsets.push(clean.len());
        }
        let spans = layout(&clean, columns);
        let rows: Vec<String> = spans
            .iter()
            .map(|span| {
                let row = &clean[span.clone()];
                if text::width(row) > columns {
                    "?".into()
                } else {
                    row.into()
                }
            })
            .collect();
        let mut last_row = 0;
        let mut last_byte = 0;
        let mut last_column = 0;
        let mut stops: Vec<Stop> = source
            .iter()
            .zip(display_offsets)
            .map(|(&index, offset)| {
                // At a soft wrap, the insertion stop belongs to the following
                // row. A discarded seam space belongs to the preceding row.
                let row = spans
                    .partition_point(|span| span.start <= offset)
                    .saturating_sub(1);
                let span = &spans[row];
                let byte = offset.saturating_sub(span.start).min(rows[row].len());
                if row != last_row || byte < last_byte {
                    last_column = text::width(&rows[row][..byte]);
                } else {
                    last_column += text::width(&rows[row][last_byte..byte]);
                }
                last_row = row;
                last_byte = byte;
                Stop {
                    index,
                    row,
                    column: last_column,
                    byte,
                    display_end: byte,
                }
            })
            .collect();
        for (i, pair) in source.windows(2).enumerate() {
            let shown = text::safe(&input[pair[0]..pair[1]]);
            let stop = &mut stops[i];
            if shown != "\n" && rows[stop.row][stop.byte..].starts_with(&shown) {
                stop.display_end = stop.byte + shown.len();
            }
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

fn layout(clean: &str, columns: usize) -> Vec<Range<usize>> {
    let mut spans = Vec::new();
    let mut offset = 0;
    for line in clean.split('\n') {
        for range in text::wrap_line(line, columns) {
            spans.push(range.start + offset..range.end + offset);
        }
        offset += line.len() + 1;
    }
    spans
}
