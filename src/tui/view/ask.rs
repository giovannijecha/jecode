//! Ask user is a sheet: below the static `? Waiting for you`, a rule titled `Question`, an
//! optional step row and the question, one blank row and its keys. A block taller than its
//! room scrolls with the cursor.

use super::panel::{self, Key, Part, key};
use crate::{
    ask::Request,
    tui::{
        ask::Panel,
        line::Line,
        state::State,
        text,
        theme::{ACCENT, DIM, HAIRLINE, MUTED, STRONG},
    },
};

mod body;
use body::Body;

/// The block's body rows, for scrolling and moving the caret.
pub(in crate::tui) struct Window {
    pub start: usize,
    pub total: usize,
    pub room: usize,
    /// The columns the typed words wrap at.
    pub text: usize,
}

struct Layout {
    body: Body,
    start: usize,
    room: usize,
    overflow: bool,
}

/// The body for `height` rows: one column narrower when a scrollbar is
/// needed, starting where the user scrolled or where the caret shows.
fn layout(request: &Request, panel: &Panel, columns: usize, height: usize) -> Layout {
    let question = &request.questions[panel.step];
    let entry = panel.entry();
    let fixed =
        3 + max_key_rows(request, panel, columns) + usize::from(request.questions.len() > 1);
    let room = height.saturating_sub(fixed).max(1);
    let mut body = body::body(question, entry, columns);
    let overflow = body.rows.len() > room && columns >= 7;
    if overflow {
        body = body::body(question, entry, columns - 1);
    }
    let last = body.rows.len().saturating_sub(room);
    let target = body
        .caret
        .map_or(body.focus.clone(), |(row, _)| row..row + 1);
    let start = panel
        .top
        .unwrap_or_else(|| target.end.saturating_sub(room).min(target.start));
    Layout {
        start: start.min(last),
        body,
        room,
        overflow,
    }
}

pub(super) fn window(state: &State) -> Option<Window> {
    let (request, panel) = state.question.as_ref().zip(state.ask_panel.as_ref())?;
    let layout = layout(
        request,
        panel,
        state.width.max(1),
        super::chrome_height(state).min(state.height),
    );
    Some(Window {
        start: layout.start,
        total: layout.body.rows.len(),
        room: layout.room,
        text: layout.body.text,
    })
}

/// The status row, the rules around the step row and the question, and the
/// keys, fitted to `height`.
pub(super) fn block(
    request: &Request,
    panel: &Panel,
    status: Line,
    columns: usize,
    height: usize,
) -> (Vec<Line>, Option<(usize, usize)>) {
    let Layout {
        body,
        start,
        room,
        overflow,
    } = layout(request, panel, columns, height);
    let title = [Line::new("Question", STRONG.bold())];
    let rule = super::composer::rule(columns, HAIRLINE, &title, &Line::default());
    let mut parts = vec![(status, Part::Chrome), (rule, Part::Chrome)];
    if request.questions.len() > 1 {
        parts.push((steps(request, panel, columns), Part::Header));
    }
    let first = parts.len();
    let total = body.rows.len();
    let end = (start + room).min(total);
    let focus = body.caret.map_or(body.focus.start, |(row, _)| row);
    for (index, row) in body.rows[start..end].iter().enumerate() {
        let at = start + index;
        let mut row = row.clone();
        if overflow {
            row = panel::fit(row, columns - 1, None);
            let (glyph, style) = panel::scrollbar(index, start, total, room);
            row.push(glyph, style);
        }
        let part = if body.focus.contains(&at) {
            Part::Focus
        } else {
            Part::Row(at.abs_diff(focus))
        };
        parts.push((row, part));
    }
    // Reserve the tallest body and the most key rows of any question, so
    // switching between them never shifts the conversation above.
    let tallest = request
        .questions
        .iter()
        .zip(&panel.entries)
        .map(|(question, entry)| body::body(question, entry, columns).rows.len())
        .max()
        .unwrap_or(0)
        .min(room);
    let keys = key_rows(request, panel, columns, panel.step);
    let key_padding = max_key_rows(request, panel, columns).saturating_sub(keys.len());
    for _ in end - start..tallest + key_padding {
        parts.push((Line::default(), Part::Space));
    }
    let caret = body
        .caret
        .filter(|(row, _)| (start..end).contains(row))
        .map(|(row, column)| (first + row - start, column));
    parts.push((Line::default(), Part::Space));
    for row in keys {
        parts.push((row, Part::Keep));
    }
    panel::fitted(parts, caret, height)
}

/// The keys where the footer was. When narrow, the movement hints leave
/// first and what still does not fit wraps, so Esc and Ctrl+D always show.
fn key_rows(request: &Request, panel: &Panel, columns: usize, step: usize) -> Vec<Line> {
    if columns < 7 {
        return Vec::new();
    }
    panel::keys(&keys(request, panel, step), columns - 4, false)
        .into_iter()
        .map(|row| row.indent("  ", MUTED))
        .collect()
}

fn max_key_rows(request: &Request, panel: &Panel, columns: usize) -> usize {
    (0..request.questions.len())
        .map(|step| key_rows(request, panel, columns, step).len())
        .max()
        .unwrap_or(0)
}

/// `☑` before each answered title, the current one in bold `accent`, and
/// `N of M` in `dim` ending two cells from the edge.
/// When the titles do not fit, earlier ones leave behind `…` so the current
/// title stays in view.
fn steps(request: &Request, panel: &Panel, columns: usize) -> Line {
    let count = format!("{} of {}", panel.step + 1, request.questions.len());
    let count_cells = text::cells(&count);
    let titles: Vec<(String, bool)> = request
        .questions
        .iter()
        .zip(&panel.entries)
        .map(|(question, entry)| {
            let title = text::clean(&question.title).replace('\n', " ");
            (title, entry.answer(question).is_some())
        })
        .collect();
    let cells = |index: usize| {
        let (title, answered) = &titles[index];
        text::cells(title)
            + if *answered && index != panel.step {
                2
            } else {
                0
            }
    };
    let room = columns.saturating_sub(count_cells + 4);
    // Titles `first..=step` with their gaps, after `…` and its gap if any.
    let mut span = (0..=panel.step).map(cells).sum::<usize>() + 3 * panel.step;
    let mut first = 0;
    while first < panel.step && span + if first > 0 { 4 } else { 0 } > room {
        span -= cells(first) + 3;
        first += 1;
    }
    let mut row = Line::new("  ", MUTED);
    if first > 0 {
        row.push("…", MUTED);
    }
    for (index, (title, answered)) in titles.iter().enumerate().skip(first) {
        if index > 0 {
            row.push("   ", MUTED);
        }
        if index == panel.step {
            row.push(title, ACCENT.bold());
        } else if *answered {
            row.push("☑ ", ACCENT);
            row.push(title, MUTED);
        } else {
            row.push(title, MUTED);
        }
    }
    let mut row = row.shortened(columns.saturating_sub(count_cells + 4));
    let used = panel::cells(&row);
    row.push(
        &" ".repeat(columns.saturating_sub(count_cells + 2 + used)),
        MUTED,
    );
    row.push(&count, DIM);
    row.shortened(columns)
}

/// Enter chooses, moves on or sends; Space selects for a multiple choice;
/// ←→ (or Tab inside typed words) move between several questions.
fn keys(request: &Request, panel: &Panel, step: usize) -> Vec<Key<'static>> {
    let question = &request.questions[step];
    let entry = &panel.entries[step];
    let several = request.questions.len() > 1;
    let typing = entry.in_text(question);
    let free = question.options.is_empty();
    let verb = if several && step + 1 < request.questions.len() {
        "next"
    } else if !several && !typing && !question.is_multiple() {
        "choose"
    } else {
        "send"
    };
    let mut keys = Vec::new();
    if question.is_multiple() && !typing {
        keys.push(key("Space", "toggle"));
    }
    if entry.row == usize::MAX {
        keys.push(key(digits(question.options.len() + 1), "choose"));
    } else {
        keys.push(key("Enter", verb));
    }
    if free {
        keys.push(key("Ctrl+J", "new line").optional());
    }
    if several {
        let switch = if typing && !entry.editor.text.is_empty() {
            key("Tab", "questions")
        } else {
            key("←→", "questions")
        };
        keys.push(switch.optional());
    } else if !free {
        keys.push(key("↑↓", "move").optional());
    }
    keys.push(key("Esc", "back to chat"));
    keys.push(key("Ctrl+D", "dismiss"));
    keys
}

/// The digits that choose a row: one key per digit, so rows past the ninth
/// are reached with ↑↓.
fn digits(rows: usize) -> &'static str {
    const RANGES: [&str; 9] = ["1", "1–2", "1–3", "1–4", "1–5", "1–6", "1–7", "1–8", "1–9"];
    RANGES[rows.clamp(1, 9) - 1]
}
