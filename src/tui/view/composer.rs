use super::{fitted, footer, information, menus, notices, panel, processes};
use crate::tui::{
    activity,
    feedback::Feedback,
    hint,
    line::Line,
    state::State,
    text,
    theme::{ACCENT, BODY, CURSOR, MUTED, Style},
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
    let notice_rows = feedback(state.notice.as_ref(), columns);
    let copy_rows = feedback(state.copy_notice.as_ref(), columns);
    // Activity and progress ride the composer rule; an open panel gives them a row.
    let activity_row = is_panel.then(|| status(state)).flatten();
    let bottom_count = notice_rows.len() + copy_rows.len() + usize::from(activity_row.is_some());
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
        super::commands::render(state, columns, panel_height)
    } else {
        draft(state, model, directory, columns, height, back_to_bottom)
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
    if let Some(status) = activity_row {
        // Panels name their own Esc action.
        extra.push(extra_row(status.shortened(extra_columns)));
    }
    extra.extend(copy_rows);
    if extra.len() > remaining {
        extra.drain(..extra.len() - remaining);
    }
    let offset = extra.len();
    extra.append(&mut composer);
    (extra, cursor.map(|(row, col)| (row + offset, col)))
}

/// `─ ▰▰▱▱▱▱ 1h 10m · Drafts 1 · Alt+↑ ──── 3/7 · esc ─`. As columns shrink,
/// the draft range leaves first, then the context takes a shorter form,
/// shortens and leaves, then `esc`, then the time; the scanner stays.
/// `contexts` lists the context's forms, longest first; `scanner` draws the
/// scanner with or without the time.
fn working_rule(
    scanner: &dyn Fn(bool) -> Line,
    contexts: &[Line],
    position: &str,
    escape: bool,
    style: Style,
    columns: usize,
) -> Line {
    let (mut range, mut escape, mut time) = (position, escape, true);
    loop {
        let mut line = Line::new("─ ", style);
        for span in scanner(time).spans {
            line.push(&span.text, span.style);
        }
        let right = [range, if escape { "esc" } else { "" }]
            .into_iter()
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>()
            .join(" · ");
        let right = if right.is_empty() {
            right
        } else {
            format!(" {right} ─")
        };
        // A space after the scanner group and at least three rule cells.
        let fixed = text::cells(&line.plain()) + 4 + text::cells(&right);
        let room = columns.saturating_sub(fixed);
        let full = contexts
            .first()
            .map_or(0, |context| 3 + panel::cells(context));
        if !range.is_empty() && (fixed > columns || full > room) {
            range = "";
            continue;
        }
        if fixed > columns && escape {
            escape = false;
            continue;
        }
        if fixed > columns && time {
            time = false;
            continue;
        }
        let whole = contexts
            .iter()
            .find(|context| 3 + panel::cells(context) <= room);
        if let Some(context) = whole.or(contexts.last()).filter(|_| room >= 9) {
            line.push(" · ", MUTED);
            line = line.append(context.shortened(room - 3));
        }
        line.push(" ", style);
        line.push(
            &"─".repeat(columns.saturating_sub(text::cells(&line.plain()) + text::cells(&right))),
            style,
        );
        line.push(&right, MUTED);
        return line.shortened(columns);
    }
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

/// Result and banner rows; progress belongs to the rule.
fn feedback(notice: Option<&Feedback>, columns: usize) -> Vec<Line> {
    notice
        .filter(|notice| !notice.in_progress())
        .map_or_else(Vec::new, |notice| notices::lines(notice, columns))
}

/// The scanner beside progress that has no turn, for its frame.
fn scanner(progress: &Feedback) -> Line {
    let started = progress.started().unwrap_or_else(std::time::Instant::now);
    activity::indicator((started.elapsed().as_millis() / 120) as usize)
}

/// The scanner, the turn's time and any progress, as one row.
fn status(state: &State) -> Option<Line> {
    let progress = state.current_progress();
    let mut line = match (&state.activity, progress) {
        (Some(activity), _) => activity.line(true),
        (None, Some(progress)) => scanner(progress),
        (None, None) => return None,
    };
    if let Some(progress) = progress {
        line.push(
            if state.activity.is_some() {
                " · "
            } else {
                " "
            },
            MUTED,
        );
        line = line.append(notices::progress(progress));
    }
    Some(line)
}

pub(super) fn information_height(state: &State) -> usize {
    let columns = state.width.max(1);
    let feedback_rows = feedback(state.notice.as_ref(), columns).len()
        + feedback(state.copy_notice.as_ref(), columns).len();
    let activity = usize::from(state.activity.is_some() || state.current_progress().is_some());
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
    back_to_bottom: bool,
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
    let progress = state.current_progress();
    let contexts = contexts(progress, &context, state.queue.len(), contextual);
    let top = if let Some(activity) = state.activity.as_ref().filter(|_| columns >= 7) {
        // Esc names itself only while it stops the turn.
        let escape = !activity.stopping && !contextual && !back_to_bottom;
        let scanner = |time| activity.line(time);
        working_rule(&scanner, &contexts, &input.position, escape, style, columns)
    } else if let Some(progress) = progress.filter(|_| columns >= 7) {
        let scanner = |_| scanner(progress);
        working_rule(&scanner, &contexts, &input.position, false, style, columns)
    } else if !context.is_empty() && columns >= 7 {
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

/// The rule's context forms, longest first: progress, then the draft context.
fn contexts(
    progress: Option<&Feedback>,
    context: &str,
    drafts: usize,
    contextual: bool,
) -> Vec<Line> {
    let context = (!context.is_empty()).then(|| Line::new(context, MUTED));
    let short = context
        .as_ref()
        .filter(|_| !contextual)
        .map(|_| Line::new(&format!("Drafts {drafts}"), MUTED));
    let Some(notice) = progress else {
        return context.into_iter().chain(short).collect();
    };
    let progress = notices::progress(notice);
    let joined = |tail: &Line| {
        let mut line = progress.clone();
        line.push(" · ", MUTED);
        line.append(tail.clone())
    };
    context
        .iter()
        .chain(short.iter())
        .map(joined)
        .chain([progress.clone()])
        .chain(notices::progress_forms(notice))
        .collect()
}

pub(super) struct Input {
    pub lines: Vec<Line>,
    pub cursor: (usize, usize),
    pub position: String,
}

pub(super) fn input(
    state: &State,
    columns: usize,
    style: Style,
    foreground: Style,
    limit: usize,
) -> Input {
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
