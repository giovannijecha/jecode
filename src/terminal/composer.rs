//! Expandable local controls within one pair of rules; metadata lives below them.
use super::{
    model::Model,
    style::{Row, Tone, lines},
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
    let path = view.workspace.as_deref().unwrap_or("Conversation only");
    let full = format!("{} · medium", view.selected.id());
    let model = if text::width(&full) + 10 <= width {
        full
    } else {
        format!(
            "{} · medium",
            match view.selected {
                crate::session::Model::Luna => "Luna",
                crate::session::Model::Terra => "Terra",
            }
        )
    };
    let model_width = text::width(&model);
    let path = path_label(path, width.saturating_sub(model_width + 2));
    let gap = width.saturating_sub(text::width(&path) + model_width);
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
    let mut draft = input(&model.editor, width, draft_height);
    if model.editor.text.is_empty() && model.menu.panel.is_some() {
        draft[0].text = "›  Filter…".into();
    }
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
    let available = capacity
        .saturating_sub(usize::from(capacity > 2))
        .min(height / 2)
        .min(16);
    let mut body = if model.menu.active(&model.editor.text) && model.account.is_some() {
        super::menu::rows(model, width, available)
    } else {
        activity(model, width, available)
    };
    if let Some(view) = &model.account {
        if view.queued > 0 && body.len() < available {
            body.push(clipped(
                &format!("{} queued · next model step", view.queued),
                width,
                Tone::Muted,
            ));
        }
        // Local errors/confirmation remain visible when a menu is still open.
        if !view.notice.is_empty()
            && model.menu.active(&model.editor.text)
            && body.len() < available
        {
            body.push(clipped(
                &view.notice,
                width,
                if view.failed {
                    Tone::Error
                } else {
                    Tone::Muted
                },
            ));
        }
    }
    let rule = "─".repeat(width);
    let mut rows = vec![Row::new(&rule, Tone::Accent)];
    if !body.is_empty() {
        rows.append(&mut body);
        if rows.len() <= capacity {
            rows.push(Row::blank());
        }
    }
    rows.extend(draft);
    rows.push(Row::new(rule, Tone::Accent));
    rows.extend(footer);
    rows
}

fn activity(model: &Model, width: usize, available: usize) -> Vec<Row> {
    let mut rows = if let Some(run) = model.account.as_ref().and_then(|v| v.command.as_ref()) {
        super::command_view::active(run, width, model.tools.reduced_motion)
    } else if let Some(demo) = &model.action_demo {
        super::action_view::active(demo, width, model.tools.reduced_motion)
    } else if model.tools.active().is_some() {
        super::tool_view::active(model, width)
    } else if let Some(view) = &model.account {
        if view.notice.is_empty() {
            Vec::new()
        } else {
            lines(
                &view.notice,
                width,
                if view.failed {
                    Tone::Error
                } else {
                    Tone::Muted
                },
            )
        }
    } else {
        let state = if model.streaming() {
            "Streaming / Esc to stop"
        } else if model.status.starts_with("Simulated") {
            "Stream failed / partial response kept"
        } else if model.status.starts_with("Interrupted") {
            "Interrupted / partial response kept"
        } else if model.status.starts_with("Preview history") {
            "Preview full / restart to continue"
        } else {
            ""
        };
        if state.is_empty() {
            Vec::new()
        } else {
            lines(state, width, Tone::Muted)
        }
    };
    rows.truncate(available);
    rows
}

fn input(editor: &text::Editor, width: usize, height: usize) -> Vec<Row> {
    let available = width - 2;
    let mut parts = vec![String::new()];
    let mut used = 0;
    let mut cursor = (0, 0);
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
    let start = cursor.0.saturating_sub(height - 1);
    parts
        .into_iter()
        .enumerate()
        .skip(start)
        .take(height)
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
