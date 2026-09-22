//! Pure inline layout. The terminal, rather than a second viewport, owns scrolling.
use super::{
    block::BlockLayout,
    model::Model,
    style::{Row, Tone, lines},
    text,
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
        let mut rows = vec![
            Row::blank(),
            Row::new("jecode", Tone::Brand),
            Row::new(&model.subtitle, Tone::Muted),
            Row::blank(),
        ];
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
    let mut rows = Vec::new();
    if height < 9 || width < 24 {
        rows.extend(
            lines("Resize terminal / Ctrl+Q exits", width, Tone::Muted)
                .into_iter()
                .take(height.saturating_sub(1)),
        );
        for row in &mut rows {
            row.transient = true;
        }
        return rows;
    }
    if let Some(demo) = model.action_demo.as_ref().filter(|demo| demo.pending()) {
        rows = super::action_view::approval(demo, width, !model.editor.text.is_empty());
        separate_input(&mut rows, height);
        for row in &mut rows {
            row.transient = true;
        }
        return rows;
    }
    if let Some(edit) = model
        .account
        .as_ref()
        .and_then(|v| v.approval.as_ref())
        .filter(|e| !e.submitted)
    {
        rows = super::action_view::decision(
            if edit.kind == super::approval_view::Kind::Command {
                "? Run this command with your user permissions?"
            } else {
                "? Apply this file change?"
            },
            edit.allow,
            width,
            !model.editor.text.is_empty(),
        );
        separate_input(&mut rows, height);
        for row in &mut rows {
            row.transient = true;
        }
        return rows;
    }
    let state = if let Some(view) = &model.account {
        (!view.notice.is_empty()).then_some((
            view.notice.as_str(),
            if view.failed {
                Tone::Error
            } else {
                Tone::Muted
            },
        ))
    } else if model.streaming() {
        Some(("Streaming / Esc to stop", Tone::Muted))
    } else if model.status.starts_with("Simulated") {
        Some(("Stream failed / partial response kept", Tone::Error))
    } else if model.status.starts_with("Interrupted") {
        Some(("Interrupted / partial response kept", Tone::Muted))
    } else if model.status.starts_with("Preview history") {
        Some(("Preview full / restart to continue", Tone::Error))
    } else {
        None
    };
    let command = model.account.as_ref().and_then(|v| v.command.as_ref());
    let active = model.tools.active().is_some() || model.action_demo.is_some() || command.is_some();
    if !active && let Some((state, tone)) = state {
        rows.extend(lines(state, width, tone));
    }
    let rule = "─".repeat(width);
    rows.push(Row::new(&rule, Tone::Accent));
    rows.extend(composer(&model.editor, width));
    rows.push(Row::new(rule, Tone::Accent));
    let hint = if width >= 60 {
        "Enter send  ·  / commands  ·  Esc stop  ·  Ctrl+Q exit"
    } else {
        "Enter send / Ctrl+Q exit"
    };
    rows.extend(lines(hint, width, Tone::Muted));
    if let Some(view) = &model.account
        && view.queued > 0
    {
        rows.push(super::tool_view::clipped(
            &format!(
                "{} queued / delivered at the next model boundary",
                view.queued
            ),
            width,
            Tone::Muted,
        ));
    }
    let available = height.saturating_sub(1 + rows.len());
    rows.splice(0..0, super::menu::rows(model, width, available));
    if active {
        let available = height.saturating_sub(1 + rows.len());
        let activity = if let Some(command) = command {
            super::command_view::active(command, width, model.tools.reduced_motion)
        } else if let Some(demo) = &model.action_demo {
            super::action_view::active(demo, width, model.tools.reduced_motion)
        } else {
            super::tool_view::active(model, width)
        };
        rows.splice(0..0, activity.into_iter().take(available));
    }
    separate_input(&mut rows, height);
    for row in &mut rows {
        row.transient = true;
    }
    rows
}

/// One gap between status/decision text and the input surface when it fits.
/// Small windows retain their controls before adding decorative whitespace.
fn separate_input(rows: &mut Vec<Row>, height: usize) {
    if rows.len() + 1 >= height {
        return;
    }
    if let Some(index) = rows
        .iter()
        .position(|r| r.tone == Tone::Accent && r.text.starts_with('─'))
        && index > 0
        && !rows[index - 1].text.is_empty()
    {
        rows.insert(index, Row::blank());
    }
}

#[cfg(test)]
pub fn frame(model: &Model, columns: usize, height: usize) -> Vec<Row> {
    Layout::default().frame(model, columns, height)
}

fn composer(editor: &text::Editor, width: usize) -> Vec<Row> {
    let available = width - 2;
    let mut parts = vec![String::new()];
    let mut used = 0;
    let mut cursor = (0, 0);
    // The cursor is a separate display cell, never a marker inserted into user text.
    let boundaries = text::boundaries(&editor.text);
    for (index, end) in boundaries
        .iter()
        .copied()
        .zip(boundaries.iter().copied().skip(1).map(Some).chain([None]))
    {
        if index == editor.cursor {
            if used == available {
                parts.push(String::new());
                used = 0;
            }
            cursor = (parts.len() - 1, parts.last().unwrap().len());
            parts.last_mut().unwrap().push(' ');
            used += 1;
        }
        if let Some(end) = end {
            let unit = &editor.text[index..end];
            let shown = if text::width(unit) > available {
                "..."
            } else {
                unit
            };
            let size = text::width(shown);
            if used + size > available {
                parts.push(String::new());
                used = 0;
            }
            parts.last_mut().unwrap().push_str(shown);
            used += size;
        }
    }
    let start = cursor.0.saturating_sub(2);
    parts
        .into_iter()
        .enumerate()
        .skip(start)
        .take(3)
        .map(|(index, text)| {
            let prefix = if index == 0 { "› " } else { "  " };
            let mut row = Row::new(format!("{prefix}{text}"), Tone::Text);
            if index == cursor.0 {
                let position = prefix.len() + cursor.1;
                row.spans.push((position..position + 1, Tone::Cursor));
            }
            if editor.text.is_empty() {
                row.text.push_str("Ask anything…");
                row.tone = Tone::Muted;
            }
            row
        })
        .collect()
}
