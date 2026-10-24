//! Read-only panels: no focus marker, the arrows scroll and values wrap in
//! their own column. Several sections show as tabs.

use super::panel::{self, Panel, Part, key};
use crate::tui::{
    information::{Information, Section},
    line::Line,
    text,
    theme::{MUTED, STRONG, TOOL_DIM, WARNING},
};

/// A label as drawn: keycaps, a command with muted arguments, or a word.
fn label(section: &Section, label: &str) -> Line {
    let label = text::clean(label);
    if section.keys {
        let mut line = Line::default();
        for (index, key) in label.split(" · ").enumerate() {
            if index > 0 {
                line.push(" ", MUTED);
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

/// Label and value rows; narrow panels stack the label above its value.
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
    let stacked = column > width / 2;
    let mut rows = Vec::new();
    for (name, value) in &section.details {
        if value.is_empty() {
            if !rows.is_empty() {
                rows.push(Line::default());
            }
            rows.push(Line::new(&text::clean(name), TOOL_DIM).shortened(width));
            continue;
        }
        let indent = if stacked { 2 } else { column };
        let lines =
            Line::new(&text::clean(value), MUTED).wrap(width.saturating_sub(indent).max(1), true);
        if stacked {
            rows.push(label(section, name).shortened(width));
        }
        for (index, line) in lines.into_iter().enumerate() {
            let lead = if index == 0 && !stacked {
                panel::fit(label(section, name), column, None)
            } else {
                Line::new(&" ".repeat(indent), MUTED)
            };
            rows.push(lead.append(line));
        }
    }
    rows
}

/// The rows, wrapped beside the scrollbar when they overflow `limit`.
fn content(info: &Information, width: usize, limit: usize) -> Vec<Line> {
    let all = rows(info, width);
    if all.len() > limit {
        rows(info, width.saturating_sub(2).max(1))
    } else {
        all
    }
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

/// Rows of content that fit beside the header, rules, spacing and keys.
fn limit(info: &Information, width: usize, height: usize) -> usize {
    let keys = ending(info, width, true).len();
    let rules = if info.sections.len() > 1 { 2 } else { 1 };
    let chrome = 1 + rules + 4 + keys;
    if height >= chrome + 3 {
        height - chrome
    } else {
        height.saturating_sub(1 + keys)
    }
    .max(1)
}

pub(super) fn render(
    info: &Information,
    columns: usize,
    height: usize,
) -> (Vec<Line>, Option<(usize, usize)>) {
    let width = panel::width(columns);
    let all = content(info, width, limit(info, width, height));
    let limit = limit(info, width, height);
    let total = all.len();
    let offset = info.offset.min(total.saturating_sub(limit));
    let overflow = total > limit;
    let section = info.current();
    let aside = Line::new(&section.aside, MUTED);
    let mut out = Panel::default();
    out.add(
        panel::header(None, &info.title, &info.context, aside, width),
        Part::Title,
    );
    if info.sections.len() > 1 {
        let names: Vec<&str> = info
            .sections
            .iter()
            .map(|section| section.name.as_str())
            .collect();
        out.extend(
            panel::tabs(&names, info.section, Line::default(), width),
            Part::Header,
        );
    } else {
        out.add(panel::divider(width), Part::Header);
    }
    out.space();
    let rows = width.saturating_sub(2 * usize::from(overflow)).max(1);
    for (index, line) in all.into_iter().enumerate().skip(offset).take(limit) {
        let bar = overflow.then_some((index - offset, offset, total, limit));
        out.add(
            panel::barred(panel::fit(line, rows, None), bar),
            Part::Row(index - offset),
        );
    }
    out.space();
    out.extend(ending(info, width, overflow), Part::Keep);
    out.finish(columns, height)
}

pub(in crate::tui) fn scroll(info: &mut Information, code: u16, columns: usize, height: usize) {
    let width = panel::width(columns);
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
    let limit = limit(info, width, height);
    let end = content(info, width, limit).len().saturating_sub(limit);
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
