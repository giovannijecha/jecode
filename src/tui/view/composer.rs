use super::{fitted, footer, information, menus, panel, processes};
use crate::tui::{
    hint,
    line::Line,
    notice,
    state::State,
    text,
    theme::{ACCENT, BODY, CURSOR, MUTED, SELECTED_SURFACE, STRONG, Style},
};

pub(super) fn render(
    state: &State,
    model: &str,
    directory: &str,
    columns: usize,
    height: usize,
    back_to_bottom: bool,
) -> (Vec<Line>, Option<(usize, usize)>) {
    let is_panel = super::panel_open(state);
    let notice_rows = state
        .notice
        .as_ref()
        .map_or_else(Vec::new, |notice| feedback(notice, columns));
    let copy_rows = state
        .copy_notice
        .as_ref()
        .map_or_else(Vec::new, |notice| feedback(notice, columns));
    let bottom_count = notice_rows.len()
        + copy_rows.len()
        + usize::from(state.activity.is_some())
        + usize::from(state.tool_focus.is_some());
    let reserve = if is_panel {
        bottom_count.min(height.saturating_sub(3))
    } else {
        0
    };
    let panel_height = height.saturating_sub(reserve);
    let footer_rows = if is_panel {
        footer::panel_rows(state, panel_height)
    } else {
        0
    };
    let panel_height = panel_height - footer_rows;
    let (mut composer, mut cursor) = if let Some(panel) = &state.processes {
        processes::render(panel, columns, panel_height)
    } else if let Some(info) = &state.information {
        information::render(info, columns, panel_height)
    } else if let Some(selector) = &state.selector {
        menus::selector(selector, columns, panel_height)
    } else if state.suggestions.panel {
        commands(state, columns, panel_height)
    } else {
        draft(state, model, directory, columns, height)
    };
    if footer_rows > 0 {
        // The footer stays on the terminal surface below an open panel.
        composer.push(footer::line(state, model, directory, columns));
    }
    if composer.len() > height {
        // On very short terminals retain the input/caret rather than overflow the viewport.
        let focus = cursor.map_or(0, |(row, _)| row);
        let start = focus.saturating_sub(height.saturating_sub(2));
        let end = (start + height).min(composer.len());
        composer = composer.drain(start..end).collect();
        cursor = cursor
            .filter(|(row, _)| *row >= start && *row < end)
            .map(|(row, col)| (row - start, col));
    }
    let remaining = height.saturating_sub(composer.len());
    let extra_columns = if is_panel {
        panel::width(columns)
    } else {
        columns
    };
    // Only contextual rows join an open panel; feedback keeps the terminal surface.
    let extra_row = |line| {
        if is_panel {
            panel::row(line, columns)
        } else {
            line
        }
    };
    // Pending drafts precede the feedback and activity chrome.
    let mut extra = Vec::new();
    let draft_menu = state
        .selector
        .as_ref()
        .is_some_and(|menu| matches!(menu.purpose, crate::tui::selector::Purpose::Drafts { .. }));
    let queue_budget = if draft_menu {
        0
    } else {
        remaining.saturating_sub(bottom_count).min(8)
    };
    let mut drafts = state.queue.drafts();
    drafts.retain(|(slot, _)| Some(*slot) != state.queue.editing());
    let pending = drafts.len();
    let visible = pending.min(queue_budget);
    let entries = drafts.into_iter().map(|(slot, text)| {
        (
            text,
            match slot {
                crate::tui::drafts::Slot::Queued(_) => "queued",
                crate::tui::drafts::Slot::Paused(_) => "paused",
            },
        )
    });
    for (index, (message, status)) in entries.take(visible).enumerate() {
        let first = message.lines().next().unwrap_or("");
        let preview = format!(
            "› {}{}",
            first,
            if message.contains('\n') { "…" } else { "" }
        );
        let label = if index == 0 && visible < pending {
            format!("{status} {}", panel::position(0, visible, pending))
        } else {
            status.into()
        };
        let preview = text::clean(&preview);
        let label = [label.as_str(), status]
            .into_iter()
            .find(|label| text::cells(label) + text::cells(&preview).min(10) + 2 <= extra_columns)
            .unwrap_or("");
        let row = if label.is_empty() {
            Line::new(&text::ellipsize(&preview, extra_columns), MUTED)
        } else {
            fitted(&preview, label, extra_columns)
        };
        extra.push(extra_row(row));
    }
    extra.extend(notice_rows);
    if let Some(activity) = &state.activity {
        let escape = if is_panel {
            "Esc closes menu"
        } else if state.queue.is_editing() {
            "Esc cancels edit"
        } else if state.history.is_browsing() || state.tool_focus.is_some() || back_to_bottom {
            "Esc returns"
        } else {
            "Esc stops"
        };
        extra.push(extra_row(activity.line(extra_columns, escape)));
    }
    extra.extend(copy_rows);
    if state.tool_focus.is_some() {
        extra.push(extra_row(
            Line::new(
                "Tools · ↑↓ select · Enter details · Esc returns",
                crate::tui::theme::TOOL_DIM,
            )
            .shortened(extra_columns),
        ));
    }
    if extra.len() > remaining {
        extra.drain(..extra.len() - remaining);
    }
    let offset = extra.len();
    extra.append(&mut composer);
    (extra, cursor.map(|(row, col)| (row + offset, col)))
}

/// Key hints for the current draft context, shown in place of the directory.
fn hints(state: &State) -> Option<&'static [&'static str]> {
    if matches!(
        state.queue.editing(),
        Some(crate::tui::drafts::Slot::Paused(_))
    ) {
        Some(&[
            "Enter save · Ctrl+S send · Esc cancel",
            "↵ save · ^S send · Esc",
            "↵ · ^S · Esc",
        ])
    } else if state.queue.is_editing() {
        Some(&["Enter save · Esc cancel", "↵ save · Esc", "↵ · Esc"])
    } else if state.history.is_browsing() {
        Some(&[
            "Enter send · Ctrl+P/N history · Esc back",
            "↵ send · ^P/N · Esc",
            "↵ · ^P/N · Esc",
        ])
    } else {
        None
    }
}

fn feedback(notice: &crate::tui::feedback::Feedback, columns: usize) -> Vec<Line> {
    notice::lines(notice.kind.into(), &notice.text, columns)
}

pub(super) fn information_height(state: &State) -> usize {
    let columns = state.width.max(1);
    let feedback_rows = [state.notice.as_ref(), state.copy_notice.as_ref()]
        .into_iter()
        .flatten()
        .map(|notice| feedback(notice, columns).len())
        .sum::<usize>();
    let activity = usize::from(state.activity.is_some());
    let height = super::chrome_height(state);
    let height = height.saturating_sub((feedback_rows + activity).min(height.saturating_sub(3)));
    height - footer::panel_rows(state, height)
}

fn draft(
    state: &State,
    model: &str,
    directory: &str,
    columns: usize,
    height: usize,
) -> (Vec<Line>, Option<(usize, usize)>) {
    let style = if state.activity.is_some() {
        MUTED
    } else {
        ACCENT
    };
    let border = "─".repeat(columns);
    let editing = state.queue.edit_index();
    let contextual = state.queue.is_editing() || state.history.is_browsing();
    let limit = height.saturating_sub(2 + usize::from(columns >= 7) + usize::from(contextual));
    let input = input(state, columns, style, BODY, limit);
    let badge = text::cells(&input.position);
    let context = if let Some(index) = editing {
        format!("Editing draft {}/{}", index + 1, state.queue.len())
    } else if state.history.is_browsing() {
        "Prompt history".into()
    } else if !state.queue.is_empty() {
        format!("Drafts {} · Alt+↑", state.queue.len())
    } else {
        String::new()
    };
    let top = if !context.is_empty() && columns >= 7 {
        let range = if badge > 0 && badge + text::cells(&context) + 7 <= columns {
            format!(" {} ─", input.position)
        } else {
            String::new()
        };
        let label = text::ellipsize(&context, columns.saturating_sub(text::cells(&range) + 4));
        let mut line = Line::new("─ ", style);
        line.push(&label, MUTED);
        line.push(" ", style);
        line.push(
            &"─".repeat(columns.saturating_sub(text::cells(&line.plain()) + text::cells(&range))),
            style,
        );
        line.push(&range, MUTED);
        line
    } else if badge > 0 && badge + 6 <= columns {
        let mut line = Line::new(&"─".repeat(columns - badge - 3), style);
        line.push(&format!(" {} ", input.position), MUTED);
        line.push("─", style);
        line
    } else {
        Line::new(&border, style)
    };
    let mut lines = vec![top];
    lines.extend(input.lines);
    lines.push(Line::new(&border, style));
    if contextual {
        lines.push(Line::new(
            &hint::choose(hints(state).unwrap_or(&[]), columns),
            MUTED,
        ));
    }
    if columns >= 7 {
        lines.push(footer::line(state, model, directory, columns));
    }
    (lines, Some((input.cursor.0 + 1, input.cursor.1)))
}

struct Input {
    lines: Vec<Line>,
    cursor: (usize, usize),
    position: String,
}

fn input(state: &State, columns: usize, style: Style, foreground: Style, limit: usize) -> Input {
    if columns < 7 {
        // At extreme widths the editor gets the entire row, including its caret.
        // The logical draft/caret are unchanged and regain multiline layout later.
        let (value, caret) = state.editor.viewport(columns, false);
        let line = if state.editor.text.is_empty() {
            Line::new(&text::clip("Ask anything…", columns), MUTED).caret(0, CURSOR)
        } else {
            Line::new(&value, foreground).caret(caret, CURSOR)
        };
        return Input {
            lines: vec![line],
            cursor: (0, caret),
            position: String::new(),
        };
    }
    let layout = state.editor.layout(columns.saturating_sub(3));
    let total = layout.rows.len();
    let limit = limit.clamp(1, 5);
    let start = layout
        .cursor
        .0
        .saturating_sub(limit / 2)
        .min(total.saturating_sub(limit));
    let end = (start + limit).min(total);
    let mut lines = Vec::new();
    let mut cursor = (0, 0);
    for index in start..end {
        let mut line = Line::new(if index == start { "› " } else { "  " }, style);
        if state.editor.text.is_empty() {
            line.push("Ask anything…", MUTED);
        } else {
            let row = &layout.rows[index];
            let mut at = 0;
            for label in &row.labels {
                line.push(&row.text[at..label.start], foreground);
                line.push(&row.text[label.clone()], ACCENT);
                at = label.end;
            }
            line.push(&row.text[at..], foreground);
        }
        if index == layout.cursor.0 {
            let column = layout.cursor.1 + 2;
            line = line.caret(column, CURSOR);
            cursor = (lines.len(), column);
        }
        lines.push(line);
    }
    Input {
        lines,
        cursor,
        position: panel::position(start, end, total),
    }
}

fn commands(state: &State, columns: usize, height: usize) -> (Vec<Line>, Option<(usize, usize)>) {
    let width = panel::width(columns);
    let reserve = 2 + usize::from(height >= 5) + usize::from(state.suggestions.visible);
    let input = input(state, width, ACCENT, STRONG, height.saturating_sub(reserve));
    let mut lines = vec![panel::heading("Commands", &input.position, width)];
    lines.extend(input.lines);
    let cursor = Some((input.cursor.0 + 1, input.cursor.1));
    if state.suggestions.visible {
        let (options, position) = menus::suggestions(
            &state.suggestions,
            width,
            height.saturating_sub(lines.len() + 3),
        );
        if input.position.is_empty() {
            lines[0] = panel::heading("Commands", &position, width);
        }
        lines.extend(options);
    }
    let selected = lines
        .iter()
        .position(|line| line.background == Some(SELECTED_SURFACE));
    let working = state.activity.is_some();
    let immediate = if state.suggestions.visible {
        matches!(
            state.suggestions.chosen(),
            Some("/copy" | "/drafts" | "/terminal")
        )
    } else {
        state
            .editor
            .text
            .split_whitespace()
            .next()
            .is_some_and(|name| {
                ["/copy", "/drafts", "/terminal"]
                    .iter()
                    .any(|command| name.eq_ignore_ascii_case(command))
            })
    };
    let action = if working && !immediate {
        "queue"
    } else {
        "run"
    };
    let controls = if state.suggestions.visible && !state.suggestions.matches.is_empty() {
        vec![
            format!("↑↓ move · Tab complete · Enter {action} · Esc close"),
            format!("↑↓ · Tab · Enter {action} · Esc close"),
            "Enter · Esc".into(),
            "↵ · Esc".into(),
        ]
    } else {
        vec![
            format!("Enter {action} · Esc close"),
            "Enter · Esc".into(),
            "↵ · Esc".into(),
        ]
    };
    let variants: Vec<&str> = controls.iter().map(String::as_str).collect();
    lines.push(Line::new(&hint::choose(&variants, width), MUTED));
    let mut priorities = vec![cursor.map_or(1, |(row, _)| row)];
    priorities.extend(selected);
    priorities.extend([lines.len() - 1, 0]);
    panel::finish(lines, cursor, &priorities, columns, height)
}
