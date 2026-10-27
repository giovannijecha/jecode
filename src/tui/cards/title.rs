use super::{Status, badge};
use crate::tui::{
    line::{Line, Origin},
    text,
    theme::{BODY, MUTED},
};

/// File cards open on ` NAME ` and the path in BODY, one space after the
/// badge; wrapped rows hang under the path while it has room, else at
/// column 2. Collapsed, the path keeps three rows.
pub(super) fn rows(
    name: &str,
    target: &str,
    status: Status,
    columns: usize,
    expanded: bool,
) -> Vec<Line> {
    let mut head = badge::badge(name, status);
    head.push(" ", BODY);
    let hang = text::cells(&head.plain());
    let indent = if columns.saturating_sub(hang) >= 24 {
        hang
    } else {
        2
    };
    let first = columns.saturating_sub(hang).max(1);
    let rest = columns.saturating_sub(indent).max(1);
    let target = text::clean(target);
    let parts: Vec<_> = target.split('\n').collect();
    let mut texts = Vec::new();
    for (index, part) in parts.iter().enumerate() {
        let width = if texts.is_empty() { first } else { rest };
        let mut wrapped: Vec<_> = words(part, width, rest)
            .into_iter()
            .map(|row| (row, false))
            .collect();
        if index + 1 < parts.len()
            && let Some((_, newline)) = wrapped.last_mut()
        {
            *newline = true;
        }
        texts.extend(wrapped);
        if !expanded && texts.len() > 3 {
            break;
        }
    }
    let cut = !expanded && texts.len() > 3;
    if cut {
        texts.truncate(3);
    }
    let count = texts.len();
    let mut character = 0;
    texts
        .into_iter()
        .enumerate()
        .map(|(index, (mut body, newline_after))| {
            let room = if index == 0 { first } else { rest };
            if cut && index + 1 == count {
                if text::cells(&body) + 1 > room {
                    body = Line::new(&body, BODY)
                        .shortened(room.saturating_sub(1))
                        .plain();
                    body.pop();
                }
                body.push('…');
            }
            let mut line = if index == 0 {
                head.clone()
            } else {
                Line::new(&" ".repeat(indent), MUTED)
            };
            line.push(&body, BODY);
            line.origin = Some(Origin { line: 0, character });
            character += body.chars().count() + usize::from(newline_after);
            line.shortened(columns)
        })
        .collect()
}

/// Word-wrapped rows: the first `first` cells wide, the rest `rest` cells.
fn words(value: &str, first: usize, rest: usize) -> Vec<String> {
    let mut wrapped = Line::new(value, BODY).into_wrapped(first, true);
    let mut rows = Vec::new();
    if let Some(row) = wrapped.next() {
        rows.push(row.plain());
    }
    wrapped.set_columns(rest);
    rows.extend(wrapped.map(|row| row.plain()));
    rows
}
