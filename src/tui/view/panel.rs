use crate::tui::{
    line::Line,
    text,
    theme::{MUTED, PROMPT, USER_BACKGROUND},
};

pub(super) fn position(start: usize, end: usize, total: usize) -> String {
    if start >= end || start == 0 && end == total {
        String::new()
    } else {
        format!("{}–{end} / {total}", start + 1)
    }
}

pub(super) fn heading(title: &str, position: &str, columns: usize) -> Line {
    let title = text::clean(title);
    let badge = text::cells(position);
    let show = !position.is_empty() && badge + text::cells(&title).min(8) + 2 <= columns;
    let budget = if show { columns - badge - 2 } else { columns };
    let mut line = Line::new(&text::ellipsize(&title, budget), PROMPT);
    if show {
        line.push(
            &" ".repeat(columns - text::cells(&line.plain()) - badge),
            MUTED,
        );
        line.push(position, MUTED);
    }
    line
}

pub(super) fn hint<'a>(variants: &[&'a str], columns: usize) -> &'a str {
    variants
        .iter()
        .copied()
        .find(|value| text::cells(value) <= columns)
        .unwrap_or_else(|| variants.last().copied().unwrap_or(""))
}

fn margin(columns: usize) -> usize {
    if columns >= 12 { 2 } else { 0 }
}

pub(super) fn width(columns: usize) -> usize {
    columns.saturating_sub(2 * margin(columns)).max(1)
}

pub(super) fn row(line: Line, columns: usize) -> Line {
    let mut line = line
        .shortened(width(columns))
        .indent(&" ".repeat(margin(columns)), MUTED);
    line.background.get_or_insert(USER_BACKGROUND);
    line
}

// Priorities refer to content rows, before optional padding. Keep their order
// on screen while allowing input, selection and confirmation to survive resize.
pub(super) fn finish(
    lines: Vec<Line>,
    cursor: Option<(usize, usize)>,
    priorities: &[usize],
    columns: usize,
    height: usize,
) -> (Vec<Line>, Option<(usize, usize)>) {
    if height == 0 {
        return (vec![], None);
    }
    let mut keep: Vec<_> = (0..lines.len()).collect();
    if lines.len() > height {
        keep.clear();
        for index in priorities.iter().copied().chain(0..lines.len()) {
            if keep.len() == height {
                break;
            }
            if index < lines.len() && !keep.contains(&index) {
                keep.push(index);
            }
        }
        keep.sort_unstable();
    }
    let padding = usize::from(keep.len() + 2 <= height);
    let cursor = cursor.and_then(|(row, column)| {
        keep.iter()
            .position(|&index| index == row)
            .map(|row| (row + padding, column + margin(columns)))
    });
    let blank = || Line::default().on(USER_BACKGROUND);
    let mut output = Vec::new();
    if padding > 0 {
        output.push(blank());
    }
    output.extend(
        keep.into_iter()
            .map(|index| row(lines[index].clone(), columns)),
    );
    if padding > 0 {
        output.push(blank());
    }
    (output, cursor)
}
