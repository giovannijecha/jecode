//! Pure inline layout. The terminal, rather than a second viewport, owns scrolling.
use super::{
    block::BlockLayout,
    model::Model,
    style::{Row, Tone, lines},
};

#[derive(Default)]
pub struct Layout {
    blocks: Vec<Option<BlockLayout>>,
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
        let mut rows = Vec::new();
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
                if !rows.is_empty() {
                    rows.push(Row::blank());
                }
                rows.extend_from_slice(rendered);
            }
        }
        self.blocks.truncate(model.blocks.len());
        // The transcript owns its single boundary with transient activity or input.
        if !rows.is_empty() {
            rows.push(Row::blank());
        }
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
        let controls = decision_footer(
            super::action_view::approval(demo, width, !model.editor.text.is_empty()),
            model,
            width,
            height,
        );
        let mut rows = activity_prefix(model, width, height, controls.len());
        rows.extend(controls);
        rows
    } else if let Some(edit) = model
        .account
        .as_ref()
        .and_then(|v| v.approval.as_ref())
        .filter(|e| !e.submitted)
    {
        let controls = decision_footer(
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
        );
        let mut rows = activity_prefix(model, width, height, controls.len());
        rows.extend(controls);
        rows
    } else {
        // Reserve the rules, draft, footer and any local controls before adding
        // runtime state. All rows share the same transient suffix on resize.
        let menu = model.account.is_some() && model.menu.active(&model.editor.text);
        let queued = model.account.as_ref().is_some_and(|view| view.queued > 0);
        let notice = !model.edit_notice.is_empty()
            || model
                .account
                .as_ref()
                .is_some_and(|view| !view.local_notice.is_empty());
        let minimum = 4 + usize::from(menu) + usize::from(queued) + usize::from(notice);
        let mut rows = activity_prefix(model, width, height, minimum);
        rows.extend(super::composer::rows(model, width, height - rows.len()));
        rows
    };
    for row in &mut rows {
        row.transient = true;
    }
    rows
}

fn activity_prefix(model: &Model, width: usize, height: usize, controls: usize) -> Vec<Row> {
    // Running panels are intrinsically compact; account notices can wrap to
    // more rows. Clip either one only to the space left for composer controls.
    let capacity = height.saturating_sub(controls + 1);
    let mut rows = super::activity_view::rows(model, width, capacity);
    if !rows.is_empty() && rows.len() + controls + 1 < height {
        rows.push(Row::blank());
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
