use super::{Line, Origin, StyledLine};
use crate::tui::theme::{CODE_TEXT, DIM};

/// A row never starts with a separator or a closer; it stays with its token.
const CLOSERS: &str = ",;)]}";
/// A row may end after a separator, a bracket or the end of a tag.
const OPENERS: &str = ",;{}()[]>";

/// How one code line folds into rows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(in crate::tui) struct Fold {
    /// At most this many rows, the last one cut with `…`.
    pub rows: Option<usize>,
    /// The first glyph shown; a later start opens the first row with `…`.
    pub from: usize,
    /// Prefer a break before a tag's `<`.
    pub markup: bool,
}

/// Glyphs `start..end` of a code line on one row, after `hang` cells.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::tui) struct Row {
    pub start: usize,
    pub end: usize,
    hang: usize,
    lead: bool,
    tail: bool,
}

/// A code line measured once for token-aware wrapping. Rows break after a
/// space or a separator, before `.` or `=` only when nothing better fits, and
/// inside a token only when it is longer than the row. Continuations hang
/// under the code's own indentation; their origins skip the hang, so every
/// row still maps to its source characters.
pub(in crate::tui) struct Code {
    styled: StyledLine,
    background: Option<crate::tui::theme::Surface>,
    origin: Option<Origin>,
    // The cells before each glyph, then the total.
    offsets: Vec<usize>,
    bytes: Vec<usize>,
}

impl Line {
    /// Every row of this code line under `fold`, each at most `width` cells.
    pub(in crate::tui) fn code(&self, width: usize, fold: Fold) -> Vec<Line> {
        let code = Code::new(self);
        code.rows(&code.layout(width, fold))
    }
}

impl Code {
    pub fn new(line: &Line) -> Self {
        let styled = line.glyphs();
        let mut offsets = Vec::with_capacity(styled.glyphs.len() + 1);
        let mut bytes = Vec::with_capacity(styled.glyphs.len());
        let (mut cells, mut byte, mut character) = (0, 0, 0);
        for glyph in &styled.glyphs {
            offsets.push(cells);
            cells += glyph.cells;
            byte += styled.characters[character..glyph.start]
                .iter()
                .map(|(character, ..)| character.len_utf8())
                .sum::<usize>();
            character = glyph.start;
            bytes.push(byte);
        }
        offsets.push(cells);
        Self {
            styled,
            background: line.background,
            origin: line.origin,
            offsets,
            bytes,
        }
    }

    pub fn len(&self) -> usize {
        self.styled.glyphs.len()
    }

    /// The glyph that starts at or after byte `offset` of the source.
    pub fn glyph(&self, offset: usize) -> usize {
        self.bytes.partition_point(|&byte| byte < offset)
    }

    pub fn cells(&self, start: usize, end: usize) -> usize {
        self.offsets[end] - self.offsets[start]
    }

    /// The first character of glyph `at`, or NUL past the end.
    pub fn at(&self, at: usize) -> char {
        self.styled
            .glyphs
            .get(at)
            .map_or('\0', |glyph| self.styled.characters[glyph.start].0)
    }

    /// Whether a row may start at glyph `at` without splitting a token.
    pub fn boundary(&self, at: usize) -> bool {
        let before = self.at(at.wrapping_sub(1));
        before == ' ' || OPENERS.contains(before) || matches!(self.at(at), '.' | '=')
    }

    pub fn layout(&self, width: usize, fold: Fold) -> Vec<Row> {
        let width = width.max(1);
        let length = self.len();
        let indent = (0..length).take_while(|&at| self.at(at) == ' ').count();
        let lead = fold.from > 0;
        // A window hides the indentation, so its rows hang under the `…`.
        let hang = (if lead { 2 } else { indent + 2 }).min(width / 2);
        let mut rows: Vec<Row> = Vec::new();
        let mut position = fold.from.min(length);
        loop {
            let first = rows.is_empty();
            if !first || lead {
                while position < length && self.at(position) == ' ' {
                    position += 1;
                }
            }
            if !first && position >= length {
                break;
            }
            let lead = first && lead;
            let mut hang = if first { 0 } else { hang };
            if position < length {
                // A glyph wider than the room beside the hang drops the hang.
                hang = hang.min(width.saturating_sub(self.cells(position, position + 1)));
            }
            let mut room = width.saturating_sub(hang + usize::from(lead));
            let last = fold.rows.is_some_and(|rows_cap| rows.len() + 1 >= rows_cap);
            if self.cells(position, length) <= room {
                rows.push(Row {
                    start: position,
                    end: length,
                    hang,
                    lead,
                    tail: false,
                });
                break;
            }
            if last {
                room = room.saturating_sub(1);
            }
            let end = self.split(position, room, last, fold.markup);
            rows.push(Row {
                start: position,
                end,
                hang,
                lead,
                tail: last && (end..length).any(|at| self.at(at) != ' '),
            });
            if last {
                break;
            }
            position = end;
        }
        rows
    }

    /// The end of a row from `start` within `room` cells. A row cut with `…`
    /// keeps a token break only when it fills 60% of the room; in markup, a
    /// break before a tag's `<` wins on the same condition.
    fn split(&self, start: usize, room: usize, cut: bool, markup: bool) -> usize {
        let length = self.len();
        let mut limit = start;
        while limit < length && self.cells(start, limit + 1) <= room {
            limit += 1;
        }
        if limit >= length {
            return length;
        }
        let first = (start..limit)
            .find(|&at| self.at(at) != ' ')
            .unwrap_or(limit);
        let (mut tag, mut good, mut fair) = (None, None, None);
        for at in first + 1..=limit {
            let (here, before) = (self.at(at), self.at(at - 1));
            if markup && here == '<' {
                tag = Some(at);
            }
            if CLOSERS.contains(here)
                // Nor inside a run of openers and quotes: `('(` stays together.
                || "([{".contains(before) && (at < 2 || "([{'\"`".contains(self.at(at - 2)))
            {
                continue;
            }
            if here == ' ' || OPENERS.contains(before) {
                good = Some(at);
            } else if matches!(here, '.' | '=') {
                fair = Some(at);
            }
        }
        let fills = |at: usize| self.cells(start, at) * 10 >= room * 6;
        if let Some(tag) = tag.filter(|&at| fills(at)) {
            return tag;
        }
        if let Some(at) = [good, fair]
            .into_iter()
            .flatten()
            .find(|&at| !cut || fills(at))
        {
            return at;
        }
        // Inside a long token; the next row still never opens with a closer.
        let mut end = limit.max(start + 1);
        while !cut && end > start + 1 && CLOSERS.contains(self.at(end)) {
            end -= 1;
        }
        end
    }

    pub fn rows(&self, layout: &[Row]) -> Vec<Line> {
        layout
            .iter()
            .map(|row| {
                let character = self
                    .styled
                    .glyphs
                    .get(row.start)
                    .map_or(self.styled.characters.len(), |glyph| glyph.start);
                let mut line = Line {
                    background: self.background,
                    origin: self.origin.map(|origin| Origin {
                        character: origin.character + character,
                        ..origin
                    }),
                    ..Line::default()
                };
                if row.hang > 0 {
                    line.push(&" ".repeat(row.hang), CODE_TEXT);
                }
                if row.lead {
                    line.push("…", DIM);
                }
                for glyph in &self.styled.glyphs[row.start..row.end] {
                    line.push_glyph(&self.styled.characters, glyph, None);
                }
                if row.tail {
                    line.push("…", DIM);
                }
                line
            })
            .collect()
    }
}
