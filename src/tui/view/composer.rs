use super::{
    agents, ask, footer, information, menus,
    panel::{self, Part, Sheet},
    processes, status, todo,
};
mod editor;
use crate::tui::{
    drafts::Slot,
    line::Line,
    state::State,
    text,
    theme::{ACCENT, CURSOR, DIM, FAINT, MUTED, STRONG, Style, WARNING},
};
use editor::input;

/// Queued drafts shown above the status row at most.
const QUEUE_ROWS: usize = 8;

/// The dock: queued drafts, the status row, then the composer with the
/// footer, its key hints or an open panel below.
pub(super) fn render(
    state: &State,
    model: &str,
    directory: &str,
    columns: usize,
    height: usize,
    back_to_bottom: bool,
) -> (Vec<Line>, Option<(usize, usize)>) {
    let contextual = state.queue.is_editing() || state.history.is_browsing();
    // A question takes the composer's place unless a panel opened over it.
    let question = state
        .question
        .as_ref()
        .zip(state.ask_panel.as_ref())
        .filter(|_| {
            state.quitting.is_none() && state.processes.is_none() && state.agents_panel.is_none()
        });
    let sheet = sheet(state, columns).filter(|_| question.is_none());
    // Esc interrupts only where nothing else holds it.
    let escape = sheet.is_none() && question.is_none() && !contextual && !back_to_bottom;
    let status = status::row(state, columns, escape);
    let (lines, cursor) = match (question, sheet) {
        (Some((request, panel)), _) => ask::block(request, panel, status, columns, height),
        (None, Some(sheet)) => docked(state, &sheet, status, columns, height),
        (None, None) => {
            let (draft, cursor) = draft(state, model, directory, columns, height.saturating_sub(1));
            let mut lines = vec![status];
            lines.extend(draft);
            let mut cursor = cursor.map(|(row, column)| (row + 1, column));
            if lines.len() > height {
                // On very short terminals retain the input/caret rather than overflow the viewport.
                let focus = cursor.map_or(0, |(row, _)| row);
                let start = focus.saturating_sub(height.saturating_sub(2));
                let end = (start + height).min(lines.len());
                lines = lines.drain(start..end).collect();
                cursor = cursor
                    .filter(|(row, _)| *row >= start && *row < end)
                    .map(|(row, col)| (row - start, col));
            }
            (lines, cursor)
        }
    };
    let mut lines = lines;
    let draft_menu = state
        .selector
        .as_ref()
        .is_some_and(|menu| matches!(menu.purpose, crate::tui::selector::Purpose::Drafts { .. }));
    let budget = if draft_menu {
        0
    } else {
        height.saturating_sub(lines.len()).min(QUEUE_ROWS)
    };
    let mut rows = queue(state, columns, budget);
    let offset = rows.len();
    rows.append(&mut lines);
    (rows, cursor.map(|(row, col)| (row + offset, col)))
}

/// The open panel, if any.
fn sheet(state: &State, columns: usize) -> Option<Sheet<'_>> {
    if let Some(work) = &state.quitting {
        Some(super::quit::sheet(state, work, columns))
    } else if let Some(panel) = &state.processes {
        Some(processes::sheet(panel, columns))
    } else if let Some(panel) = &state.agents_panel {
        Some(agents::sheet(state, panel, columns))
    } else if let Some(panel) = &state.todo_panel {
        Some(todo::sheet(state, panel, columns))
    } else if let Some(info) = &state.information {
        Some(information::sheet(info, columns))
    } else if let Some(selector) = &state.selector {
        Some(menus::sheet(selector, columns))
    } else if state.suggestions.panel {
        Some(super::commands::sheet(state, columns))
    } else {
        None
    }
}

/// The status row and the composer above a panel: the field or the draft
/// between the rules, the panel's title on the bottom rule.
fn top(state: &State, sheet: &Sheet, status: Line, columns: usize) -> Rows {
    let style = rule_style(state);
    let mut parts = vec![(
        status,
        if state.question.is_some() {
            Part::Keep
        } else {
            Part::Chrome
        },
    )];
    let mut caret = None;
    if let Some(field) = &sheet.field {
        let label = [Line::new(&field.label, MUTED)];
        parts.push((rule(columns, style, &label, &Line::default()), Part::Chrome));
        // At extreme widths the field needs every cell, as the draft does.
        let prefix = if columns < 7 { "" } else { "› " };
        let mut line = Line::new(prefix, ACCENT.bold());
        if field.value.is_empty() {
            line.push(field.placeholder, MUTED);
        } else {
            line.push(&field.value, STRONG);
        }
        line.push(field.suffix, MUTED);
        let column = (text::cells(prefix) + field.caret).min(columns.saturating_sub(1));
        caret = Some((parts.len(), column));
        parts.push((line.shortened(columns).caret(column, CURSOR), Part::Focus));
    } else {
        let input = input(
            state,
            columns,
            ACCENT,
            STRONG,
            window(state),
            sheet.draft_caret,
        );
        let position = Line::new(&input.position, MUTED);
        parts.push((
            rule(columns, style, &draft_label(state), &position),
            Part::Chrome,
        ));
        if sheet.draft_caret {
            caret = Some((parts.len() + input.cursor.0, input.cursor.1));
        }
        parts.extend(input.lines.into_iter().map(|line| (line, Part::Draft)));
    }
    parts.push((rule(columns, style, &sheet.title, &sheet.aside), Part::Keep));
    Rows { parts, caret }
}

struct Rows {
    parts: Vec<(Line, Part)>,
    caret: Option<(usize, usize)>,
}

/// Rows the panel's body gets below `top` rows within `height`.
fn room(state: &State, sheet: &Sheet, top: usize, height: usize) -> usize {
    let fixed = top + sheet.head.len() + 1 + (sheet.tail)(true).len();
    panel::room(state.height, height, fixed)
}

/// The rows an open panel's body shows at once.
pub(super) fn panel_room(state: &State) -> usize {
    let columns = state.width.max(1);
    sheet(state, columns).map_or(0, |sheet| {
        measured(
            state,
            &sheet,
            Line::default(),
            columns,
            super::chrome_height(state).min(state.height),
        )
        .1
    })
}

/// The dock with a panel: the composer, then the panel's head, body, one
/// blank row and its keys, fitted to `height` by what matters most.
fn docked(
    state: &State,
    sheet: &Sheet,
    status: Line,
    columns: usize,
    height: usize,
) -> (Vec<Line>, Option<(usize, usize)>) {
    let (rows, _) = measured(state, sheet, status, columns, height);
    panel::fitted(rows.parts, rows.caret, height)
}

/// Use the fitter's own retained body rows as the panel's visible capacity.
/// A shorter body or wrapped keys may free or take rows; recompute until the
/// body window and its navigation use the same capacity.
fn measured(
    state: &State,
    sheet: &Sheet,
    status: Line,
    columns: usize,
    height: usize,
) -> (Rows, usize) {
    let top = top(state, sheet, status.clone(), columns).parts.len();
    let mut capacity = room(state, sheet, top, height);
    loop {
        let (rows, body) = dock_parts(state, sheet, status.clone(), columns, capacity);
        let visible = panel::kept_indices(&rows.parts, rows.caret, height)
            .into_iter()
            .filter(|index| body.contains(index))
            .count()
            .min(capacity);
        if visible >= capacity || visible == 0 {
            return (rows, visible);
        }
        capacity = visible;
    }
}

fn dock_parts(
    state: &State,
    sheet: &Sheet,
    status: Line,
    columns: usize,
    capacity: usize,
) -> (Rows, std::ops::Range<usize>) {
    let Rows { mut parts, caret } = top(state, sheet, status, columns);
    let body = (sheet.body)(capacity);
    let overflow = sheet.total.is_none_or(|total| total > capacity);
    let spaced = !(sheet.head.is_empty() && body.is_empty());
    parts.extend(sheet.head.iter().cloned().map(|line| (line, Part::Header)));
    let first = parts.len();
    parts.extend(body);
    let body = first..parts.len();
    if spaced {
        parts.push((Line::default(), Part::Space));
    }
    let tail = if sheet.confirming {
        Part::Focus
    } else {
        Part::Keep
    };
    // Wrapped keys keep their last row first: it holds the way out.
    let keys = (sheet.tail)(overflow);
    let last = keys.len().saturating_sub(1);
    parts.extend(keys.into_iter().enumerate().map(|(index, line)| {
        let part = if index == last || sheet.confirming {
            tail
        } else {
            Part::Row(0)
        };
        (panel::indent(line, columns), part)
    }));
    (Rows { parts, caret }, body)
}

/// A panel's dock alone, below an empty composer, for panel tests.
#[cfg(test)]
pub(super) fn panel_rows(sheet: &Sheet, columns: usize, height: usize) -> Vec<Line> {
    let state = State {
        width: columns,
        height,
        ..State::default()
    };
    docked(&state, sheet, Line::default(), columns, height).0
}

/// The rules turn faint while a turn works or retries.
fn rule_style(state: &State) -> Style {
    if state.activity.is_some() {
        FAINT
    } else {
        ACCENT
    }
}

/// The top rule's label: the draft being edited, the history, or the drafts.
fn draft_label(state: &State) -> Vec<Line> {
    if let Some(index) = state.queue.edit_index() {
        vec![Line::new(
            &format!("Editing draft {}/{}", index + 1, state.queue.len()),
            MUTED,
        )]
    } else if state.history.is_browsing() {
        vec![Line::new("Prompt history", MUTED)]
    } else if !state.queue.is_empty() {
        let drafts = state.queue.len();
        vec![
            Line::new(&format!("Drafts {drafts} · Alt+↑"), MUTED),
            Line::new(&format!("Drafts {drafts}"), MUTED),
        ]
    } else {
        Vec::new()
    }
}

/// `› first line…` in `muted`, `queued` in `dim` or `paused` in `warning`
/// ending two cells from the edge; the first label carries the range when
/// not every draft fits.
fn queue(state: &State, columns: usize, budget: usize) -> Vec<Line> {
    let mut drafts = state.queue.drafts();
    drafts.retain(|(slot, _)| Some(*slot) != state.queue.editing());
    let pending = drafts.len();
    let visible = pending.min(budget);
    let end = columns.saturating_sub(2);
    drafts
        .into_iter()
        .take(visible)
        .enumerate()
        .map(|(index, (slot, message))| {
            let (status, style) = match slot {
                Slot::Queued(_) => ("queued", DIM),
                Slot::Paused(_) => ("paused", WARNING),
            };
            let first = message.lines().next().unwrap_or("");
            let more = if message.contains('\n') { "…" } else { "" };
            let preview = text::clean(&format!("› {first}{more}"));
            let ranged = (index == 0 && visible < pending)
                .then(|| format!("{status} {}", panel::position(0, visible, pending)));
            let label = ranged
                .iter()
                .map(String::as_str)
                .chain([status])
                .find(|label| text::cells(label) + text::cells(&preview).min(10) + 2 <= end)
                .unwrap_or("");
            if label.is_empty() {
                return Line::new(&text::ellipsize(&preview, columns), MUTED);
            }
            let room = end - text::cells(label) - 2;
            let preview = text::ellipsize(&preview, room);
            let mut row = Line::new(&preview, MUTED);
            row.push(
                &" ".repeat(end - text::cells(&preview) - text::cells(label)),
                MUTED,
            );
            row.push(label, style);
            row
        })
        .collect()
}

/// A full-width rule with an optional `─ label` and an aside before ` ─` on
/// the right; the aside drops first, then the label shortens.
pub(super) fn rule(columns: usize, style: Style, label: &[Line], aside: &Line) -> Line {
    let tail = |aside: &Line| {
        if aside.spans.is_empty() {
            Line::default()
        } else {
            let mut line = Line::new(" ", style);
            line = line.append(aside.clone());
            line.push(" ─", style);
            line
        }
    };
    let head = |label: &Line| {
        let mut line = Line::new("─ ", style);
        line = line.append(label.clone());
        line.push(" ", style);
        line
    };
    let fits = |head: &Line, tail: &Line| panel::cells(head) + panel::cells(tail) + 2 <= columns;
    let finish = |head: Line, tail: Line| {
        let fill = columns.saturating_sub(panel::cells(&head) + panel::cells(&tail));
        let mut line = head;
        line.push(&"─".repeat(fill), style);
        line.append(tail).shortened(columns)
    };
    if label.is_empty() {
        let tail = tail(aside);
        return if fits(&Line::default(), &tail) {
            finish(Line::default(), tail)
        } else {
            Line::new(&"─".repeat(columns), style)
        };
    }
    // The aside drops before the label loses detail.
    for head in label.iter().map(head) {
        for tail in [tail(aside), Line::default()] {
            if fits(&head, &tail) {
                return finish(head, tail);
            }
        }
    }
    let last = label.last().unwrap();
    let room = columns.saturating_sub(5);
    if room < 3 {
        return Line::new(&"─".repeat(columns), style);
    }
    finish(head(&last.shortened(room)), Line::default())
}

/// The draft's window: a third of the screen, five to ten rows.
fn window(state: &State) -> usize {
    (state.height / 3).clamp(5, 10)
}

/// Key hints in place of the footer while a draft edit or the prompt
/// history holds the composer.
fn hints(state: &State) -> Option<&'static [panel::Key<'static>]> {
    const PAUSED: &[panel::Key] = &[
        panel::key("Enter", "save"),
        panel::key("Ctrl+S", "send"),
        panel::key("Esc", "cancel"),
    ];
    const EDITING: &[panel::Key] = &[panel::key("Enter", "save"), panel::key("Esc", "cancel")];
    const HISTORY: &[panel::Key] = &[
        panel::key("Enter", "send"),
        panel::key("Ctrl+P/N", "history").optional(),
        panel::key("Esc", "back"),
    ];
    if matches!(state.queue.editing(), Some(Slot::Paused(_))) {
        Some(PAUSED)
    } else if state.queue.is_editing() {
        Some(EDITING)
    } else if state.history.is_browsing() {
        Some(HISTORY)
    } else {
        None
    }
}

fn draft(
    state: &State,
    model: &str,
    directory: &str,
    columns: usize,
    height: usize,
) -> (Vec<Line>, Option<(usize, usize)>) {
    let style = rule_style(state);
    let limit = height.saturating_sub(2 + usize::from(columns >= 7));
    let input = input(state, columns, ACCENT, STRONG, limit, true);
    let position = Line::new(&input.position, MUTED);
    let mut lines = vec![rule(columns, style, &draft_label(state), &position)];
    lines.extend(input.lines);
    lines.push(Line::new(&"─".repeat(columns), style));
    if columns >= 7 {
        lines.push(match hints(state) {
            Some(keys) => {
                let keys = panel::keys(keys, columns.saturating_sub(4), false);
                keys.into_iter()
                    .next()
                    .unwrap_or_default()
                    .indent("  ", MUTED)
            }
            None => footer::line(state, model, directory, columns),
        });
    }
    (lines, Some((input.cursor.0 + 1, input.cursor.1)))
}
