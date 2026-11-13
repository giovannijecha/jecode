use super::Layout;
use std::{ops::Range, rc::Rc};

#[derive(Debug, Default)]
pub(super) struct Cache {
    source: String,
    columns: usize,
    labels: Vec<String>,
    references: Option<Option<usize>>,
    chips: Vec<Range<usize>>,
    layout: Rc<Layout>,
}

impl Cache {
    /// `references` holds the start of the reference being typed when the editor
    /// draws reference chips.
    pub fn get(
        &mut self,
        source: &str,
        cursor: usize,
        columns: usize,
        labels: Vec<String>,
        references: Option<Option<usize>>,
    ) -> Rc<Layout> {
        let columns = columns.max(4);
        let rebuild = self.columns != columns
            || self.labels != labels
            || self.references.is_some() != references.is_some()
            || self.layout.rows.is_empty();
        let edited = self.source != source;
        if rebuild || edited || self.references != references {
            let chips =
                references.map_or_else(Vec::new, |typing| super::chips::chips(source, typing));
            if rebuild {
                self.layout = Rc::new(Layout::new(source, cursor, columns, &labels, &chips));
            } else {
                let common = if edited {
                    self.source
                        .bytes()
                        .zip(source.bytes())
                        .take_while(|(before, after)| before == after)
                        .count()
                } else {
                    source.len()
                };
                // A changed chip may start before the edit: closing a quoted path, or a
                // typed reference ending, restyles text on earlier rows.
                let from =
                    first_change(&self.chips, &chips).map_or(common, |start| start.min(common));
                if edited || from < common {
                    // Start before the edit boundary so combining marks and joins can
                    // extend the preceding glyph, including at a wrapped row boundary.
                    let row = self
                        .layout
                        .rows
                        .partition_point(|row| row.positions[0].0 < from)
                        .saturating_sub(1);
                    Rc::make_mut(&mut self.layout).reflow(source, row, columns, &labels, &chips);
                }
            }
            self.columns = columns;
            self.labels = labels;
            self.references = references;
            self.chips = chips;
            self.source.clear();
            self.source.push_str(source);
        }
        if self.layout.cursor != self.layout.cursor_position(cursor) {
            Rc::make_mut(&mut self.layout).move_cursor(cursor);
        }
        Rc::clone(&self.layout)
    }
}

/// The start of the first chip that differs between two layouts of a draft.
fn first_change(before: &[Range<usize>], after: &[Range<usize>]) -> Option<usize> {
    let index = before
        .iter()
        .zip(after)
        .position(|(before, after)| before != after)
        .unwrap_or(before.len().min(after.len()));
    match (before.get(index), after.get(index)) {
        (Some(before), Some(after)) => Some(before.start.min(after.start)),
        (Some(chip), None) | (None, Some(chip)) => Some(chip.start),
        (None, None) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::first_change;
    use std::ops::Range;

    fn chips(ranges: &[(usize, usize)]) -> Vec<Range<usize>> {
        ranges.iter().map(|&(start, end)| start..end).collect()
    }

    #[test]
    fn the_first_changed_chip_marks_where_reflow_starts() {
        let two = chips(&[(1, 4), (8, 12)]);
        let one = chips(&[(1, 4)]);
        assert_eq!(first_change(&two, &two), None);
        assert_eq!(first_change(&two, &chips(&[(1, 4), (9, 13)])), Some(8));
        assert_eq!(first_change(&one, &two), Some(8));
        assert_eq!(first_change(&two, &one), Some(8));
        assert_eq!(first_change(&chips(&[(5, 9)]), &chips(&[(0, 9)])), Some(0));
    }
}
