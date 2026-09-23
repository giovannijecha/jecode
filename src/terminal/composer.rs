//! Editable input and local controls within one pair of rules; metadata lives below.
use super::{
    editor::Editor,
    editor_visual::Visual,
    model::Model,
    style::{Row, Tone},
    text,
    tool_view::clipped,
};

pub(super) fn metadata(model: &Model, width: usize) -> Vec<Row> {
    let Some(view) = &model.account else {
        return vec![clipped(
            "Local demo · no model connected",
            width,
            Tone::Muted,
        )];
    };
    let path = view.directory.as_deref().unwrap_or("Conversation only");
    let effort = view.selected.effort().unwrap_or("provider default");
    let full = format!("{} · {effort}", view.selected.id());
    let model = if text::width(&full) + 10 <= width {
        full
    } else {
        let suffix = view
            .selected
            .id()
            .rsplit('-')
            .next()
            .unwrap_or(view.selected.id());
        let id_budget = width.saturating_sub(effort.len() + 5);
        let suffix = &suffix[suffix.len().saturating_sub(id_budget)..];
        format!("…{suffix} · {effort}")
    };
    let usable = width.saturating_sub(1); // renderer reserves the last column
    let model_width = text::width(&model);
    let path = path_label(path, usable.saturating_sub(model_width + 1));
    let gap = usable.saturating_sub(text::width(&path) + model_width);
    vec![clipped(
        &format!("{path}{}{model}", " ".repeat(gap)),
        width,
        Tone::Muted,
    )]
}

fn path_label(path: &str, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    let path = text::safe(path).replace('\n', " ");
    if text::width(&path) <= width {
        return path;
    }
    if let Some(name) = path.rsplit(['/', '\\']).next()
        && text::width(name) <= width
    {
        return name.into();
    }
    let mut start = path.len();
    let mut used = 1; // Ellipsis plus complete display units from the end.
    for pair in text::boundaries(&path).windows(2).rev() {
        used += text::width(&path[pair[0]..pair[1]]);
        if used > width {
            break;
        }
        start = pair[0];
    }
    format!("…{}", &path[start..])
}

struct Area {
    draft: Vec<Row>,
    footer: Vec<Row>,
    capacity: usize,
}
fn area(model: &Model, width: usize, height: usize) -> Area {
    let footer = metadata(model, width);
    let panel = model.menu.active(&model.editor.text);
    // Rules and footer stay visible. A tall draft must leave room for a local panel.
    let available = height.saturating_sub(1 + footer.len() + 2);
    let draft_height = available
        .saturating_sub(if panel { 2 } else { 0 })
        .clamp(1, 3);
    let draft = input(
        &model.editor,
        width,
        draft_height,
        panel && model.account.is_some(),
        model.menu.panel.is_some(),
    );
    let capacity = available.saturating_sub(draft.len());
    Area {
        draft,
        footer,
        capacity,
    }
}
pub(super) fn rows(model: &Model, width: usize, height: usize) -> Vec<Row> {
    let Area {
        draft,
        footer,
        capacity,
    } = area(model, width, height);
    let available = capacity.min(height / 2).min(16);
    let menu = model.menu.active(&model.editor.text) && model.account.is_some();
    let queued = model.account.as_ref().is_some_and(|view| view.queued > 0);
    let notice = !model.edit_notice.is_empty()
        || model
            .account
            .as_ref()
            .is_some_and(|view| !view.local_notice.is_empty());
    let feedback = usize::from(queued) + usize::from(notice);
    let menu_height = available.saturating_sub(feedback.min(available.saturating_sub(1)));
    let mut body = if menu {
        super::menu::rows(model, width, menu_height)
    } else {
        Vec::new()
    };
    if let Some(view) = &model.account {
        if view.queued > 0 && body.len() < available {
            body.push(clipped(
                &format!("{} queued · next model step", view.queued),
                width,
                Tone::Muted,
            ));
        }
        if !view.local_notice.is_empty() && body.len() < available {
            body.push(clipped(
                &view.local_notice,
                width,
                if view.local_failed {
                    Tone::Error
                } else {
                    Tone::Muted
                },
            ));
        }
    } else if !model.edit_notice.is_empty() && body.len() < available {
        body.push(clipped(model.edit_notice, width, Tone::Error));
    }
    let rule = "─".repeat(width);
    let mut rows = vec![Row::new(&rule, Tone::Accent)];
    if !body.is_empty() {
        rows.append(&mut body);
    }
    rows.extend(draft);
    rows.push(Row::new(rule, Tone::Accent));
    rows.extend(footer);
    rows
}

fn input(editor: &Editor, width: usize, height: usize, menu: bool, filter: bool) -> Vec<Row> {
    let available = width.saturating_sub(2).max(1);
    // Reserve one cell for a block at a line end; wrapping never depends on
    // which editing boundary the cursor currently occupies.
    let layout = Visual::new(&editor.text, available.saturating_sub(1).max(1));
    let cursor = layout.stop(editor.cursor);
    let marked_end = (cursor.display_end > cursor.byte).then(|| {
        if editor.text[editor.cursor..].starts_with('\t') {
            cursor.byte + 1 // A tab occupies several blank cells; mark its first.
        } else {
            cursor.display_end // Highlight a whole displayed grapheme, including wide ones.
        }
    });
    let start = cursor.row.saturating_sub(height - 1);
    layout
        .rows
        .into_iter()
        .enumerate()
        .skip(start)
        .take(height)
        .map(|(index, mut shown)| {
            let prefix = if index == 0 && !menu { "› " } else { "  " };
            let cursor_span = if index == cursor.row {
                let position = cursor.byte.min(shown.len());
                let end = marked_end.unwrap_or_else(|| {
                    shown.insert(position, ' ');
                    position + 1
                });
                Some(prefix.len() + position..prefix.len() + end)
            } else {
                None
            };
            let mut row = Row::new(format!("{prefix}{shown}"), Tone::Text);
            if let Some(span) = cursor_span {
                row.spans.push((span, Tone::Cursor));
            }
            if editor.text.is_empty() {
                row.text.push_str(if filter {
                    "Filter…"
                } else {
                    "Ask anything…"
                });
                row.tone = Tone::Muted;
            }
            row
        })
        .collect()
}
