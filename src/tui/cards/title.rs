use crate::tui::{
    line::{Line, Origin},
    text,
    theme::{ACCENT, EMPHASIS, KEYWORD, MUTED, Style, TOOL_ACCENT},
};

/// The tool name takes its family colour: reads blue, changes violet and
/// commands blue-gray. The marker keeps the status.
pub(super) fn family(name: &str) -> Style {
    match name {
        "Read" | "Skill" | "Web" | "Browser" => ACCENT,
        "Update" | "Write" | "Checkpoint" => KEYWORD,
        "Bash" | "Process" => TOOL_ACCENT,
        _ => EMPHASIS,
    }
}

/// `● Name(target)`. Words wrap under the open paren, each command line takes
/// its own row, and a compact title keeps three rows ending in `…)`.
pub(super) fn rows(
    name: &str,
    target: &str,
    marker: Style,
    columns: usize,
    expanded: bool,
) -> Vec<Line> {
    let named = family(name).bold();
    let head = format!("{name}(");
    let hang = 2 + text::cells(&head);
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
        let closing = if index + 1 == parts.len() { ")" } else { "" };
        let width = if texts.is_empty() { first } else { rest };
        texts.extend(words(&format!("{part}{closing}"), width, rest));
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
        .map(|(index, mut body)| {
            let room = if index == 0 { first } else { rest };
            if cut && index + 1 == count {
                if text::cells(&body) + 2 > room {
                    body = Line::new(&body, EMPHASIS)
                        .shortened(room.saturating_sub(1))
                        .plain();
                    body.pop();
                }
                body.push_str("…)");
            }
            let mut line = if index == 0 {
                Line::new(&head, named)
            } else {
                Line::default()
            };
            match body.strip_suffix(')').filter(|_| index + 1 == count) {
                Some(open) => {
                    line.push(open, EMPHASIS);
                    line.push(")", named);
                }
                None => line.push(&body, EMPHASIS),
            }
            line.origin = Some(Origin { line: 0, character });
            character += body.chars().count();
            let gutter = if index == 0 {
                "● ".to_owned()
            } else {
                " ".repeat(indent)
            };
            line.gutter(&gutter, if index == 0 { marker } else { MUTED })
                .shortened(columns)
        })
        .collect()
}

/// Word-wrapped rows: the first `first` cells wide, the rest `rest` cells.
fn words(value: &str, first: usize, rest: usize) -> Vec<String> {
    let mut wrapped = Line::new(value, EMPHASIS).into_wrapped(first, true);
    let mut rows = Vec::new();
    if let Some(row) = wrapped.next() {
        rows.push(row.plain());
    }
    wrapped.set_columns(rest);
    rows.extend(wrapped.map(|row| row.plain()));
    rows
}
