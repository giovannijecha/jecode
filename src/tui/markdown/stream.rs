use super::{Fence, Line, Rows, backslash_break, list_item, quoted, setext, table, text};

// An append may change the last source line, turn the preceding line into a
// setext heading or table, or extend an open table. Keep only the prefix before
// that unstable region. A fence checkpoint carries its lexer state with it.
pub(in crate::tui) struct Stream {
    raw: String,
    rows: Rows,
    trailing: String,
    checkpoint: Checkpoint,
    previous_plain: Option<(usize, usize)>,
    ready: bool,
}

struct Checkpoint {
    position: usize,
    rendered: usize,
    fenced: Option<Fence>,
}

impl Stream {
    pub fn new(value: &str, columns: usize) -> Self {
        let mut cleaned = text::clean(value);
        let trailing = cleaned.split_off(cleaned.trim_end().len());
        let rows = Rows::from_clean(cleaned, columns);
        Self {
            raw: value.into(),
            rows,
            trailing,
            checkpoint: Checkpoint {
                position: 0,
                rendered: 0,
                fenced: None,
            },
            previous_plain: None,
            ready: false,
        }
    }

    // Returns the number of previously rendered rows that remain valid.
    pub fn update(&mut self, value: &str, columns: usize) -> Option<usize> {
        if columns == self.rows.columns && value == self.raw {
            return None;
        }
        if columns != self.rows.columns || !value.starts_with(&self.raw) {
            *self = Self::new(value, columns);
            return Some(0);
        }
        let suffix = text::clean(&value[self.raw.len()..]);
        self.raw.clear();
        self.raw.push_str(value);
        self.rows.value.push_str(&self.trailing);
        self.rows.value.push_str(&suffix);
        self.trailing = self.rows.value.split_off(self.rows.value.trim_end().len());
        self.rows.position = self.checkpoint.position;
        self.rows.fenced = self.checkpoint.fenced.clone();
        self.rows.table = None;
        self.rows.empty_pending = self.rows.value.is_empty();
        self.previous_plain = None;
        self.ready = false;
        Some(self.checkpoint.rendered)
    }

    pub fn advance(&mut self, lines: &mut Vec<Line>, max_rows: usize) {
        if self.ready {
            return;
        }
        let mut budget = max_rows.max(1);
        while budget > 0 {
            let before = self.rows.table.is_none().then(|| Checkpoint {
                position: self.rows.position,
                rendered: lines.len(),
                fenced: self.rows.fenced.clone(),
            });
            let Some(next) = self.rows.next() else {
                self.ready = true;
                break;
            };
            if let Some(before) = before {
                let previous = self.previous_plain;
                self.previous_plain = before
                    .fenced
                    .is_none()
                    .then_some((before.position, before.rendered));
                self.checkpoint = match previous {
                    Some((position, rendered))
                        if before.fenced.is_none()
                            && self.rewind_previous(before.position, position) =>
                    {
                        Checkpoint {
                            position,
                            rendered,
                            fenced: None,
                        }
                    }
                    _ => before,
                };
            } else {
                self.previous_plain = None;
            }
            budget = budget.saturating_sub(next.len().max(1));
            lines.extend(next);
        }
    }

    // A final bare quote may gain text and join the previous quote. A line
    // provisionally underlined as a heading or table header may instead join
    // the preceding list item or quote when the delimiter changes.
    fn rewind_previous(&self, current: usize, previous: usize) -> bool {
        let value = &self.rows.value;
        if value[..current]
            .strip_suffix('\n')
            .is_some_and(backslash_break)
        {
            return true;
        }
        let Some((line, next)) = table::source_line(value, current) else {
            return false;
        };
        let Some((prior, _)) = table::source_line(value, previous) else {
            return false;
        };
        let prior = prior.trim_start();
        if quoted(prior).is_some_and(|body| !body.trim().is_empty())
            && quoted(line.trim_start()).is_some_and(|body| body.trim().is_empty())
        {
            return true;
        }
        if (quoted(prior).is_none() && list_item(prior).is_none())
            || line.trim().is_empty()
            || table::block_start(line)
        {
            return false;
        }
        table::source_line(value, next).is_some_and(|(below, after)| {
            setext(below) || table::Table::new(line, below, after, self.rows.columns, 0).is_some()
        })
    }

    pub fn ready(&self) -> bool {
        self.ready
    }
}
