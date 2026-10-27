//! Read-only panels: no focus marker, the arrows scroll and values wrap in
//! their own column. Several sections show as tabs.

use super::panel::{self, Part, Sheet, key};
use crate::tui::{
    information::{Information, Section},
    line::Line,
    text,
    theme::{DIM, MUTED, STRONG, WARNING},
};

/// A label as drawn: keycaps, a command with muted arguments, or a word.
fn label(section: &Section, label: &str) -> Line {
    let label = text::clean(label);
    if section.keys {
        let mut line = Line::default();
        for (index, key) in label.split(" · ").enumerate() {
            if index > 0 {
                line.push(" · ", DIM);
            }
            line.spans.extend(panel::keycap(key).spans);
        }
        return line;
    }
    let trimmed = label.trim_start();
    let indent = &label[..label.len() - trimmed.len()];
    let mut line = Line::new(indent, MUTED);
    if trimmed.starts_with('/') {
        let (name, arguments) = trimmed.split_once(' ').unwrap_or((trimmed, ""));
        line.push(name, STRONG);
        if !arguments.is_empty() {
            line.push(&format!(" {arguments}"), MUTED);
        }
    } else {
        line.push(
            trimmed,
            if trimmed == "Skipped" {
                WARNING
            } else {
                STRONG
            },
        );
    }
    line
}

/// Label and value rows at the text width: a label column (the widest
/// label plus two, at most half the width) with the value wrapping beside
/// it; a longer label stacks above its value.
fn rows(info: &Information, width: usize) -> Vec<Line> {
    let section = info.current();
    let column = section
        .details
        .iter()
        .filter(|(_, value)| !value.is_empty())
        .map(|(name, _)| panel::cells(&label(section, name)))
        .max()
        .unwrap_or(0)
        + 2;
    let column = column.min(width / 2);
    let mut rows = Vec::new();
    for (name, value) in &section.details {
        if value.is_empty() {
            rows.push(Line::new(&text::clean(name), DIM).shortened(width));
            continue;
        }
        let label = label(section, name);
        let stacked = panel::cells(&label) + 2 > column;
        let indent = if stacked { 2 } else { column };
        let lines =
            Line::new(&text::clean(value), MUTED).wrap(width.saturating_sub(indent).max(1), true);
        if stacked {
            rows.push(label.clone().shortened(width));
        }
        for (index, line) in lines.into_iter().enumerate() {
            let lead = if index == 0 && !stacked {
                panel::fit(label.clone(), column, None)
            } else {
                Line::new(&" ".repeat(indent), MUTED)
            };
            rows.push(lead.append(line));
        }
    }
    rows
}

fn ending(info: &Information, width: usize, overflow: bool) -> Vec<Line> {
    let mut keys = vec![key("↑↓", "scroll").scrolling()];
    if info.sections.len() > 1 {
        keys.push(key("←→", "section"));
    }
    keys.extend([
        key("PgUp/PgDn", "page").scrolling().optional(),
        key("Esc", "close"),
    ]);
    panel::keys(&keys, width, overflow)
}

pub(super) fn sheet(info: &Information, columns: usize) -> Sheet<'_> {
    let width = panel::text_width(columns);
    let head = if info.sections.len() > 1 {
        let names: Vec<&str> = info
            .sections
            .iter()
            .map(|section| section.name.as_str())
            .collect();
        panel::tabs(&names, info.section, Line::default(), columns).to_vec()
    } else {
        Vec::new()
    };
    let total = rows(info, width).len();
    Sheet {
        title: panel::title(None, &info.title, &info.context),
        aside: Line::new(&info.current().aside, MUTED),
        field: None,
        draft_caret: false,
        head,
        body: Box::new(move |room| {
            let all = rows(info, width);
            let offset = info.offset.min(total.saturating_sub(room));
            let shown = all
                .into_iter()
                .skip(offset)
                .take(room)
                .map(|line| (panel::indent(line, columns), None))
                .collect();
            panel::window(shown, offset, total, room, columns)
                .into_iter()
                .enumerate()
                .map(|(index, line)| (line, Part::Row(index)))
                .collect()
        }),
        total: Some(total),
        tail: Box::new(move |overflow| ending(info, width, overflow)),
        confirming: false,
    }
}

pub(in crate::tui) fn scroll(info: &mut Information, code: u16, columns: usize, room: usize) {
    if matches!(code, 37 | 39) {
        let count = info.sections.len();
        info.section = if code == 37 {
            (info.section + count - 1) % count
        } else {
            (info.section + 1) % count
        };
        info.offset = 0;
        return;
    }
    let end = rows(info, panel::text_width(columns))
        .len()
        .saturating_sub(room);
    info.offset = info.offset.min(end);
    info.offset = match code {
        38 => info.offset.saturating_sub(1),
        40 => info.offset.saturating_add(1).min(end),
        33 => info.offset.saturating_sub(room),
        34 => info.offset.saturating_add(room).min(end),
        36 => 0,
        35 => end,
        _ => info.offset,
    };
}
