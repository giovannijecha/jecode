use super::panel;
use crate::tui::{information::Information, line::Line, text, theme::MUTED};

fn rows(info: &Information, width: usize) -> Vec<Line> {
    let labels = info
        .details
        .iter()
        .map(|(label, _)| text::cells(&text::clean(label)))
        .max()
        .unwrap_or(0);
    let aligned = labels + 2 < width / 2;
    let mut rows = Vec::new();
    for (label, value) in &info.details {
        let label = text::clean(label);
        let value = text::clean(value);
        if aligned {
            let prefix = labels + 2;
            for (index, row) in Line::new(&value, MUTED)
                .wrap(width - prefix, true)
                .into_iter()
                .enumerate()
            {
                rows.push(row.indent(
                    &if index == 0 {
                        format!("{label}{}", " ".repeat(prefix - text::cells(&label)))
                    } else {
                        " ".repeat(prefix)
                    },
                    MUTED,
                ));
            }
        } else {
            rows.extend(Line::new(&label, MUTED).wrap(width, true));
            rows.extend(Line::new(&value, MUTED).wrap(width, true));
        }
    }
    rows
}

fn capacity(height: usize) -> usize {
    height.saturating_sub(if height >= 5 { 4 } else { 2 })
}

pub(super) fn render(
    info: &Information,
    columns: usize,
    height: usize,
) -> (Vec<Line>, Option<(usize, usize)>) {
    let width = panel::width(columns);
    let rows = rows(info, width);
    let limit = capacity(height);
    let offset = info.offset.min(rows.len().saturating_sub(limit));
    let end = (offset + limit).min(rows.len());
    let mut lines = vec![panel::heading(
        &info.title,
        &panel::position(offset, end, rows.len()),
        width,
    )];
    lines.extend(rows.into_iter().skip(offset).take(limit));
    lines.push(Line::new(
        panel::hint(
            &[
                "↑↓ scroll · PgUp/PgDn page · Esc close · type to return",
                "↑↓ · PgUp/PgDn · Esc close · type to return",
                "↑↓ scroll · Esc close",
                "↑↓ · Esc",
                "Esc",
            ],
            width,
        ),
        MUTED,
    ));
    panel::finish(
        lines,
        None,
        &[end.saturating_sub(offset) + 1, 0],
        columns,
        height,
    )
}

pub(in crate::tui) fn scroll(info: &mut Information, code: u16, columns: usize, height: usize) {
    let limit = capacity(height).max(1);
    let end = rows(info, panel::width(columns))
        .len()
        .saturating_sub(limit);
    info.offset = info.offset.min(end);
    info.offset = match code {
        38 => info.offset.saturating_sub(1),
        40 => info.offset.saturating_add(1).min(end),
        33 => info.offset.saturating_sub(limit),
        34 => info.offset.saturating_add(limit).min(end),
        36 => 0,
        35 => end,
        _ => info.offset,
    };
}
