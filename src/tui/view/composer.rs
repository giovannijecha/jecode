use super::{
    agents, ask, footer, information, menus,
    panel::{self, Part, Sheet},
    permission, processes, status, todo,
};
mod editor;
mod field;
use crate::tui::{
    line::Line,
    state::State,
    text,
    theme::{ACCENT, DIM, HAIRLINE, MUTED, STRONG, Style, WARNING},
};
use editor::input;

/// The dock has one owner: the pending draft, the status row, then the composer with
/// the footer or its key hints, or a sheet in the composer's place. A sheet never touches
/// the draft, which waits above it as `draft kept`.
pub(super) fn render(
    state: &State,
    model: &str,
    directory: &str,
    columns: usize,
    height: usize,
) -> (Vec<Line>, Option<(usize, usize)>) {
    let contextual = state.history.is_browsing();
    // A permission request holds the keys until a choice; only the quit
    // question goes above it.
    if let Some(pending) = state
        .permission
        .as_ref()
        .filter(|_| state.quitting.is_none())
    {
        let status = status::row(state, columns, false);
        let room = height.saturating_sub(kept_row(state, true));
        let (rows, cursor) =
            permission::block(pending, state.permission_level, status, columns, room);
        return above(state, rows, cursor, true, columns, height);
    }
    // A question takes the composer's place unless a panel opened over it.
    let question = state
        .question
        .as_ref()
        .zip(state.ask_panel.as_ref())
        .filter(|_| {
            state.quitting.is_none() && state.processes.is_none() && state.agents_panel.is_none()
        });
    let sheet = sheet(state, columns).filter(|_| question.is_none());
    // Only completion keeps the composer in view.
    let hidden = question.is_some() || sheet.as_ref().is_some_and(|sheet| !sheet.draft_caret);
    let room = height.saturating_sub(kept_row(state, hidden));
    // Esc interrupts only where nothing else holds it.
    let escape = sheet.is_none() && question.is_none() && !contextual;
    let status = status::row(state, columns, escape);
    let (lines, cursor) = match (question, sheet) {
        (Some((request, panel)), _) => ask::block(request, panel, status, columns, room),
        (None, Some(sheet)) => docked(state, &sheet, status, columns, room),
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
    above(state, lines, cursor, hidden, columns, height)
}

/// The row a hidden draft holds back from the sheet. A short window lets it go first.
fn kept_row(state: &State, hidden: bool) -> usize {
    usize::from(hidden && state.height >= 12 && !state.editor.text.is_empty())
}

/// The rows waiting above the dock's owner, while they fit: the queued draft, and the
/// one being written when a sheet hides the composer, which goes first.
fn above(
    state: &State,
    mut lines: Vec<Line>,
    cursor: Option<(usize, usize)>,
    hidden: bool,
    columns: usize,
    height: usize,
) -> (Vec<Line>, Option<(usize, usize)>) {
    let mut room = height.saturating_sub(lines.len());
    let kept = (hidden && !state.editor.text.is_empty() && room > 0)
        .then(|| pending_row(&state.editor.text, "draft kept", DIM, columns));
    room -= usize::from(kept.is_some());
    let queued = pending(state, columns).filter(|_| room > 0);
    let mut rows: Vec<Line> = queued.into_iter().chain(kept).collect();
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

/// What a sheet starts with: the status row, the title rule and its field. Completion
/// keeps the composer, with its title on the bottom rule, because the draft is the query.
fn top(state: &State, sheet: &Sheet, status: Line, columns: usize) -> Rows {
    let mut parts = vec![(
        status,
        if state.question.is_some() {
            Part::Keep
        } else {
            Part::Chrome
        },
    )];
    let mut caret = None;
    if sheet.draft_caret {
        let input = input(state, columns, ACCENT, STRONG, window(state), true);
        let position = Line::new(&input.position, MUTED);
        parts.push((
            rule(columns, HAIRLINE, &draft_label(state), &position),
            Part::Chrome,
        ));
        caret = Some((parts.len() + input.cursor.0, input.cursor.1));
        parts.extend(input.lines.into_iter().map(|line| (line, Part::Draft)));
        parts.push((
            rule(columns, HAIRLINE, &sheet.title, &sheet.aside),
            Part::Keep,
        ));
        return Rows { parts, caret };
    }
    parts.push((
        rule(columns, HAIRLINE, &sheet.title, &sheet.aside),
        Part::Keep,
    ));
    if let Some(field) = &sheet.field {
        let (rows, cursor) = field::rows(field, columns, window(state));
        caret = Some((parts.len() + cursor.0, cursor.1));
        parts.extend(rows.into_iter().enumerate().map(|(index, line)| {
            let part = if index == cursor.0 {
                Part::Focus
            } else {
                Part::Draft
            };
            (line, part)
        }));
    }
    Rows { parts, caret }
}

struct Rows {
    parts: Vec<(Line, Part)>,
    caret: Option<(usize, usize)>,
}

/// Rows the panel's body gets below `top` rows within `height`.
fn room(state: &State, sheet: &Sheet, top: usize, height: usize) -> usize {
    let gap = usize::from(!sheet.draft_caret && sheet.field.is_some());
    let fixed = top + gap + sheet.head.len() + 1 + (sheet.tail)(true).len();
    panel::room(state.height, height, fixed)
}

/// The rows an open panel's body shows at once.
pub(super) fn panel_room(state: &State) -> usize {
    let columns = state.width.max(1);
    sheet(state, columns).map_or(0, |sheet| {
        let height = super::chrome_height(state).min(state.height);
        let kept = kept_row(state, !sheet.draft_caret);
        measured(
            state,
            &sheet,
            Line::default(),
            columns,
            height.saturating_sub(kept),
        )
        .1
    })
}

/// The dock with a sheet: its top, then the panel's head, body, one blank row and its
/// keys, fitted to `height` by what matters most.
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
    let field = !sheet.draft_caret && sheet.field.is_some();
    let filled = !(sheet.head.is_empty() && body.is_empty());
    let spaced = field || filled;
    if field && filled {
        parts.push((Line::default(), Part::Space));
    }
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

/// The top rule's label: the history, or the pending draft with its key
/// while the composer is empty.
fn draft_label(state: &State) -> Vec<Line> {
    if state.history.is_browsing() {
        vec![Line::new("Prompt history", MUTED)]
    } else if state.queue.is_empty() {
        Vec::new()
    } else if state.editor.text.is_empty() {
        vec![Line::new("Draft · ↑", MUTED), Line::new("Draft", MUTED)]
    } else {
        vec![Line::new("Draft", MUTED)]
    }
}

fn pending(state: &State, columns: usize) -> Option<Line> {
    let (message, paused) = state.queue.pending()?;
    Some(if paused {
        pending_row(&message, "paused", WARNING, columns)
    } else {
        pending_row(&message, "queued", DIM, columns)
    })
}

/// `› first line…` in `muted`, the state ending two cells from the edge.
fn pending_row(message: &str, label: &str, style: Style, columns: usize) -> Line {
    let end = columns.saturating_sub(2);
    let first = message.lines().next().unwrap_or("");
    let more = if message.contains('\n') { "…" } else { "" };
    let preview = text::clean(&format!("› {first}{more}"));
    if text::cells(label) + text::cells(&preview).min(10) + 2 > end {
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

/// Key hints in place of the footer while the prompt history holds the
/// composer.
fn hints(state: &State) -> Option<&'static [panel::Key<'static>]> {
    const HISTORY: &[panel::Key] = &[
        panel::key("Enter", "send"),
        panel::key("Ctrl+P/N", "history").optional(),
        panel::key("Esc", "back"),
    ];
    state.history.is_browsing().then_some(HISTORY)
}

fn draft(
    state: &State,
    model: &str,
    directory: &str,
    columns: usize,
    height: usize,
) -> (Vec<Line>, Option<(usize, usize)>) {
    let style = HAIRLINE;
    let limit = height.saturating_sub(2 + usize::from(columns >= 7));
    let input = input(state, columns, ACCENT, STRONG, limit, true);
    let position = Line::new(&input.position, MUTED);
    let mut lines = vec![rule(columns, style, &draft_label(state), &position)];
    lines.extend(input.lines);
    // The level stays beside the draft as its chip. A draft also names the commands key,
    // which leaves first when narrow, because the rule would drop the level first.
    let level = crate::tui::cards::level(state.permission_level);
    let mut hint = Line::new("Ctrl+G", MUTED.bold());
    hint.push(" commands", DIM);
    let named = !state.editor.text.is_empty()
        && !state.history.is_browsing()
        && panel::cells(&hint) + panel::cells(&level) + 10 <= columns;
    let label: &[Line] = if named {
        std::slice::from_ref(&hint)
    } else {
        &[]
    };
    lines.push(rule(columns, style, label, &level));
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
