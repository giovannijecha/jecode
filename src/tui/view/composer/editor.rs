//! Rendering for the chat and question editors.

use super::{panel, window};
use crate::tui::{
    editor::Editor,
    line::Line,
    state::State,
    text,
    theme::{BADGE_IDLE, CURSOR, LIT, MUTED, Style},
};

pub(super) struct Input {
    pub lines: Vec<Line>,
    pub cursor: (usize, usize),
    pub position: String,
}

/// The draft's visible window and caret, shared by chat and Ask user.
pub(super) fn input(
    state: &State,
    columns: usize,
    marker: Style,
    foreground: Style,
    limit: usize,
    focused: bool,
) -> Input {
    input_editor(
        &state.editor,
        "Ask anything…",
        columns,
        marker,
        foreground,
        limit.clamp(1, window(state)),
        focused,
    )
}

pub(super) fn input_editor(
    editor: &Editor,
    placeholder: &str,
    columns: usize,
    marker: Style,
    foreground: Style,
    limit: usize,
    focused: bool,
) -> Input {
    let caret = |line: Line, column: usize| {
        if focused {
            line.caret(column, CURSOR)
        } else {
            line
        }
    };
    if columns < 7 {
        let (value, column) = editor.viewport(columns, false);
        let line = if editor.text.is_empty() {
            caret(Line::new(&text::clip(placeholder, columns), MUTED), 0)
        } else {
            caret(Line::new(&value, foreground), column)
        };
        return Input {
            lines: vec![line],
            cursor: (0, column),
            position: String::new(),
        };
    }
    let layout = editor.layout(columns.saturating_sub(3));
    let total = layout.rows.len();
    let limit = limit.max(1);
    let start = layout
        .cursor
        .0
        .saturating_sub(limit / 2)
        .min(total.saturating_sub(limit));
    let end = (start + limit).min(total);
    let mut lines = Vec::new();
    let mut cursor = (0, 0);
    for index in start..end {
        let mut line = if index == start {
            Line::new("› ", marker.bold())
        } else {
            Line::new("  ", marker)
        };
        if editor.text.is_empty() {
            line.push(&text::ellipsize(placeholder, columns - 3), MUTED);
        } else {
            let row = &layout.rows[index];
            let mut at = 0;
            for label in &row.labels {
                line.push(&row.text[at..label.start], foreground);
                line.push_on(&row.text[label.clone()], LIT, BADGE_IDLE);
                at = label.end;
            }
            line.push(&row.text[at..], foreground);
        }
        if index == layout.cursor.0 {
            let column = layout.cursor.1 + 2;
            line = caret(line, column);
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
