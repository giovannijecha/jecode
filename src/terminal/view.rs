//! Pure inline layout. The terminal, rather than a second viewport, owns scrolling.
use super::{
    block::BlockLayout,
    model::Model,
    style::{Row, Tone, lines},
};

#[derive(Default)]
pub struct Layout {
    blocks: Vec<Option<BlockLayout>>,
    header_width: Option<usize>,
}
impl Layout {
    pub fn frame(&mut self, model: &Model, columns: usize, height: usize) -> Vec<Row> {
        if columns < 2 || height == 0 {
            return Vec::new();
        }
        // Use the host terminal's padding; reserve only the final cell against autowrap.
        let width = columns - 1;
        // A displayed block keeps its original hard line boundaries. The terminal
        // reflows those rows on resize; regenerating them would replay scrollback.
        let header_width = *self.header_width.get_or_insert(width);
        let mut rows = vec![Row::blank(), Row::new("jecode", Tone::Brand), Row::blank()];
        rows = rows
            .into_iter()
            .flat_map(|row| lines(&row.text, header_width, row.tone))
            .collect();
        let mut groups = model.tools.groups.iter().peekable();
        for (index, block) in model.blocks.iter().enumerate() {
            if index == self.blocks.len() {
                self.blocks.push(None);
            }
            while groups.peek().is_some_and(|group| group.range.end <= index) {
                groups.next();
            }
            let block = if let Some(group) = groups.peek().filter(|g| g.range.contains(&index)) {
                if index != group.range.start {
                    continue;
                }
                let Some(summary) = &group.summary else {
                    continue;
                };
                summary
            } else {
                block
            };
            let rendered = self.blocks[index]
                .get_or_insert_with(|| BlockLayout::new(width))
                .rows(block);
            let rendered = content_rows(rendered);
            if !rendered.is_empty() {
                rows.extend_from_slice(rendered);
                // A tool-only model request has no visible assistant block.
                // Separators belong to displayed content, not request count.
                rows.push(Row::blank());
            }
        }
        self.blocks.truncate(model.blocks.len());
        rows.extend(chrome(model, columns, height));
        rows
    }
}

/// The layout owns block separation. Source-edge whitespace must not add a
/// second gap; styled code and user-panel padding remain part of their content.
fn content_rows(rows: &[Row]) -> &[Row] {
    let visible = |row: &Row| {
        matches!(
            row.tone,
            Tone::User | Tone::Code | Tone::Added | Tone::Removed
        ) || !row.text.trim().is_empty()
    };
    let Some(start) = rows.iter().position(visible) else {
        return &[];
    };
    let end = rows.iter().rposition(visible).unwrap();
    &rows[start..=end]
}

/// Current-width UI, independent of transcript layout and its original widths.
pub fn chrome(model: &Model, columns: usize, height: usize) -> Vec<Row> {
    if columns < 2 || height == 0 {
        return Vec::new();
    }
    let width = columns - 1;
    let mut rows = if height < 9 || width < 24 {
        lines("Resize terminal / Ctrl+Q exits", width, Tone::Muted)
            .into_iter()
            .take(height.saturating_sub(1))
            .collect()
    } else if let Some(demo) = model.action_demo.as_ref().filter(|demo| demo.pending()) {
        decision_footer(
            super::action_view::approval(demo, width, !model.editor.text.is_empty()),
            model,
            width,
            height,
        )
    } else if let Some(edit) = model
        .account
        .as_ref()
        .and_then(|v| v.approval.as_ref())
        .filter(|e| !e.submitted)
    {
        decision_footer(
            super::action_view::decision(
                if edit.kind == super::approval_view::Kind::Command {
                    "? Run this command with your user permissions?"
                } else {
                    "? Apply this file change?"
                },
                edit.allow,
                width,
                !model.editor.text.is_empty(),
            ),
            model,
            width,
            height,
        )
    } else {
        super::composer::rows(model, width, height)
    };
    for row in &mut rows {
        row.transient = true;
    }
    rows
}

fn decision_footer(mut rows: Vec<Row>, model: &Model, width: usize, height: usize) -> Vec<Row> {
    let footer = super::composer::metadata(model, width);
    if rows.len() + footer.len() < height {
        let after = rows.iter().rposition(|r| r.text.starts_with('─')).unwrap() + 1;
        rows.splice(after..after, footer);
    }
    rows
}

#[cfg(test)]
pub fn frame(model: &Model, columns: usize, height: usize) -> Vec<Row> {
    Layout::default().frame(model, columns, height)
}
