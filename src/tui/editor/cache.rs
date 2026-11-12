use super::Layout;
use std::rc::Rc;

#[derive(Debug, Default)]
pub(super) struct Cache {
    source: String,
    columns: usize,
    labels: Vec<String>,
    layout: Rc<Layout>,
}

impl Cache {
    pub fn get(
        &mut self,
        source: &str,
        cursor: usize,
        columns: usize,
        labels: Vec<String>,
    ) -> Rc<Layout> {
        let columns = columns.max(4);
        if self.columns != columns || self.labels != labels || self.layout.rows.is_empty() {
            self.layout = Rc::new(Layout::new(source, cursor, columns, &labels));
            self.columns = columns;
            self.labels = labels;
            self.source.clear();
            self.source.push_str(source);
        } else if self.source != source {
            let mut common = self
                .source
                .bytes()
                .zip(source.bytes())
                .take_while(|(before, after)| before == after)
                .count();
            // Closing a quoted reference or editing its end can change chips
            // on earlier wrapped rows of the same logical line.
            let line = source.as_bytes()[..common]
                .iter()
                .rposition(|byte| *byte == b'\n')
                .map_or(0, |index| index + 1);
            if source.as_bytes()[line..common]
                .iter()
                .any(|byte| matches!(byte, b'$' | b'@'))
            {
                common = line;
            }
            // Start before the edit boundary so combining marks and joins can
            // extend the preceding glyph, including at a wrapped row boundary.
            let row = self
                .layout
                .rows
                .partition_point(|row| row.positions[0].0 < common)
                .saturating_sub(1);
            Rc::make_mut(&mut self.layout).reflow(source, row, columns, &self.labels);
            self.source.clear();
            self.source.push_str(source);
        }
        if self.layout.cursor != self.layout.cursor_position(cursor) {
            Rc::make_mut(&mut self.layout).move_cursor(cursor);
        }
        Rc::clone(&self.layout)
    }
}
