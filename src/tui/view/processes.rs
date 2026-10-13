use super::panel;
use crate::tools::processes::Process;
use crate::tui::{
    line::Line,
    processes::{Logs, Panel, STREAMS},
    text,
    theme::{ACCENT, BAD, BODY, GOOD, MUTED, PROMPT, SELECTED_BACKGROUND, WARNING},
};

pub(super) fn render(
    panel: &Panel,
    columns: usize,
    height: usize,
) -> (Vec<Line>, Option<(usize, usize)>) {
    let width = panel::width(columns);
    let (lines, priorities) = match &panel.logs {
        Some(logs) => output(panel, logs, width, height),
        None => list(panel, width, height),
    };
    panel::finish(lines, None, &priorities, columns, height)
}

/// Rows left for list entries or log text once the heading, details, hint
/// and padding fit.
fn capacity(height: usize, fixed: usize) -> usize {
    height.saturating_sub(fixed + 3).max(1)
}

pub(in crate::tui) fn list_capacity(height: usize) -> usize {
    capacity(height, 1)
}

fn list(panel: &Panel, width: usize, height: usize) -> (Vec<Line>, Vec<usize>) {
    let total = panel.processes.len();
    let limit = list_capacity(height);
    let start = panel.selected.saturating_sub(limit - 1);
    let end = (start + limit).min(total);
    let mut lines = vec![panel::heading(
        "Processes",
        &panel::position(start, end, total),
        width,
    )];
    let labels = panel
        .processes
        .iter()
        .map(|process| text::cells(&process.label()))
        .max()
        .unwrap_or(0);
    let mut selected = None;
    for (index, process) in panel.processes.iter().enumerate().take(end).skip(start) {
        let chosen = index == panel.selected;
        if chosen {
            selected = Some(lines.len());
        }
        let mut line = Line::new(
            if chosen { "› " } else { "  " },
            if chosen { ACCENT } else { MUTED },
        );
        let label = process.label();
        line.push(&label, status_style(process));
        line.push(&" ".repeat(labels - text::cells(&label) + 2), MUTED);
        let room = width.saturating_sub(text::cells(&line.plain()));
        line.push(
            &text::ellipsize(&single_line(&process.command), room),
            PROMPT,
        );
        if chosen {
            line.background = Some(SELECTED_BACKGROUND);
        }
        lines.push(line);
    }
    if total == 0 {
        lines.push(Line::new(
            "No persistent processes in this conversation",
            MUTED,
        ));
    }
    let controls: &[&str] = if total == 0 {
        &["Esc close", "Esc"]
    } else {
        &[
            "↑↓ move · Enter output · Esc close · processes keep running",
            "↑↓ move · Enter output · Esc close",
            "↑↓ · Enter · Esc",
            "↵ · Esc",
        ]
    };
    lines.push(Line::new(panel::hint(controls, width), MUTED));
    let mut priorities: Vec<usize> = selected.into_iter().collect();
    priorities.extend([lines.len() - 1, 0]);
    (lines, priorities)
}

fn header(process: Option<&Process>, logs: &Logs, width: usize) -> Vec<Line> {
    let mut status = match process {
        Some(process) => Line::new(&process.label(), status_style(process)),
        None => Line::new("unavailable", BAD),
    };
    status.push(&format!(" · {}", logs.id), MUTED);
    let mut lines = vec![status.shortened(width)];
    if let Some(error) = process.and_then(|process| process.error.as_deref()) {
        lines.push(Line::new(&single_line(error), BAD).shortened(width));
    }
    let view = &logs.views[logs.stream];
    let mut tabs = Line::default();
    for (index, name) in STREAMS.into_iter().enumerate() {
        if index > 0 {
            tabs.push("  ", MUTED);
        }
        tabs.push(name, if index == logs.stream { ACCENT } else { MUTED });
    }
    let mode = if view.follow { "following" } else { "paused" };
    let used = text::cells(&tabs.plain());
    if used + text::cells(mode) + 2 <= width {
        tabs.push(&" ".repeat(width - used - text::cells(mode)), MUTED);
        tabs.push(mode, MUTED);
    }
    lines.push(tabs);
    lines
}

/// The log rows of `text` wrapped by character to `width` cells.
pub(in crate::tui) fn rows(text: &str, width: usize) -> Vec<Line> {
    let text = text::clean(text);
    if text.is_empty() {
        return vec![];
    }
    let text = text.strip_suffix('\n').unwrap_or(&text);
    if width == 1 {
        let mut rows = Vec::new();
        let mut character = 0;
        let mut empty = true;
        for glyph in text::glyphs(text) {
            if glyph.text == "\n" {
                if empty {
                    let mut line = Line::default().at_source(0);
                    line.origin.as_mut().unwrap().character = character;
                    rows.push(line);
                }
                empty = true;
            } else {
                let mut line =
                    Line::new(if glyph.cells > 1 { "…" } else { glyph.text }, BODY).at_source(0);
                line.origin.as_mut().unwrap().character = character;
                rows.push(line);
                empty = false;
            }
            character += glyph.text.chars().count();
        }
        if empty {
            let mut line = Line::default().at_source(0);
            line.origin.as_mut().unwrap().character = character;
            rows.push(line);
        }
        rows
    } else {
        Line::new(text, BODY).at_source(0).wrap(width, false)
    }
}

pub(in crate::tui) fn row_starts(text: &str, width: usize) -> Vec<usize> {
    rows(text, width)
        .into_iter()
        .map(|line| line.origin.unwrap().character)
        .collect()
}

pub(in crate::tui) fn row_at(starts: &[usize], anchor: usize) -> usize {
    starts
        .partition_point(|&start| start <= anchor)
        .saturating_sub(1)
}

/// Total rows of the open stream and the rows visible at once.
pub(in crate::tui) fn metrics(panel: &Panel, width: usize, height: usize) -> (usize, usize) {
    let Some(logs) = &panel.logs else {
        return (0, list_capacity(height));
    };
    let fixed = header(panel.selected_process(), logs, width).len() + 1;
    (
        rows(&logs.views[logs.stream].text(), width).len(),
        capacity(height, fixed),
    )
}

fn output(panel: &Panel, logs: &Logs, width: usize, height: usize) -> (Vec<Line>, Vec<usize>) {
    let process = panel.selected_process();
    let header = header(process, logs, width);
    let limit = capacity(height, header.len() + 1);
    let view = &logs.views[logs.stream];
    let rows = rows(&view.text(), width);
    let bottom = rows.len().saturating_sub(limit);
    let offset = if view.follow {
        bottom
    } else {
        rows.iter()
            .rposition(|line| {
                line.origin
                    .is_some_and(|origin| origin.character <= view.anchor)
            })
            .unwrap_or(0)
            .min(bottom)
    };
    let end = (offset + limit).min(rows.len());
    let title = process.map_or_else(|| "Process".into(), |process| single_line(&process.command));
    let mut lines = vec![panel::heading(
        &title,
        &panel::position(offset, end, rows.len()),
        width,
    )];
    lines.extend(header);
    let body = lines.len();
    if let Some(error) = &view.error {
        lines.push(Line::new(&single_line(error), BAD).shortened(width));
    }
    if rows.is_empty() && view.error.is_none() {
        let finished = process.is_none_or(|process| !process.active());
        lines.push(Line::new(
            if finished {
                "No output"
            } else {
                "No output yet"
            },
            MUTED,
        ));
    }
    let remaining = limit.saturating_sub(lines.len() - body);
    lines.extend(
        rows.into_iter()
            .skip(offset)
            .take(end - offset)
            .take(remaining),
    );
    let other = STREAMS[1 - logs.stream];
    let controls = if view.follow {
        [
            format!("↑↓ PgUp/PgDn scroll · Home start · Tab {other} · Esc back"),
            format!("↑↓ scroll · Tab {other} · Esc back"),
            "↑↓ · Tab · Esc".into(),
            "Esc".into(),
        ]
    } else {
        [
            format!("↑↓ PgUp/PgDn scroll · End follow · Tab {other} · Esc back"),
            format!("↑↓ scroll · End follow · Tab {other} · Esc back"),
            "↑↓ · End · Tab · Esc".into(),
            "Esc".into(),
        ]
    };
    let controls: Vec<&str> = controls.iter().map(String::as_str).collect();
    lines.push(Line::new(panel::hint(&controls, width), MUTED));
    let tabs = body - 1;
    let priorities = vec![body, 0, tabs, lines.len() - 1, 1];
    (lines, priorities)
}

fn status_style(process: &Process) -> &'static str {
    match (process.status, process.exit_code) {
        ("running", _) => GOOD,
        ("stopping" | "timed_out", _) => WARNING,
        ("exited", Some(0)) | ("stopped", _) => MUTED,
        _ => BAD,
    }
}

fn single_line(value: &str) -> String {
    text::clean(value).replace('\n', " ")
}
