//! The `/terminal` panel in the menu standard: the process list is a menu,
//! stopping asks in place of the key row, and output runs on the code
//! surface from column 2 to the edge, with Output and Errors as tabs.

use super::panel::{self, Key, Part, Sheet, key};
use crate::tools::processes::Process;
use crate::tui::{
    line::Line,
    processes::{Logs, Panel, STREAMS},
    text,
    theme::{BAD, BODY, CODE_SURFACE, GOOD, MUTED, STRONG, Style, WARNING},
};

pub(super) fn sheet(panel: &Panel, columns: usize) -> Sheet<'_> {
    match &panel.logs {
        Some(logs) => output(panel, logs, columns),
        None => list(panel, columns),
    }
}

/// Log text width on the code surface: column 3 to two cells before the
/// edge, leaving the last column to the scrollbar.
pub(in crate::tui) fn text_width(columns: usize) -> usize {
    columns.saturating_sub(5).max(1)
}

/// The stop confirmation, or the key row it replaces.
fn ending(panel: &Panel, keys: &[Key], width: usize, overflow: bool) -> Vec<Line> {
    match &panel.confirm_stop {
        Some(process) if process.active() => panel::confirm(
            "Stop this process?",
            &[key("Enter", "stop"), key("Esc", "cancel")],
            width,
        ),
        Some(_) => {
            let mut rows = vec![Line::new("Already finished", MUTED)];
            rows.extend(panel::keys(
                &[key("Enter", "back"), key("Esc", "cancel")],
                width,
                false,
            ));
            rows
        }
        None => panel::keys(keys, width, overflow),
    }
}

fn list(panel: &Panel, columns: usize) -> Sheet<'_> {
    let total = panel.processes.len();
    let aside = if total == 0 {
        String::new()
    } else {
        format!("{total} running")
    };
    let keys: Vec<Key> = if total == 0 {
        vec![key("Esc", "close")]
    } else {
        vec![
            key("↑↓", "move").optional(),
            key("Enter", "output"),
            key("Ctrl+D", "stop"),
            key("Esc", "close"),
        ]
    };
    let width = panel::text_width(columns);
    Sheet {
        title: panel::title(None, "Background processes", "this conversation"),
        aside: Line::new(&aside, MUTED),
        field: None,
        draft_caret: false,
        head: Vec::new(),
        body: Box::new(move |room| processes(panel, columns, room)),
        total: None,
        tail: Box::new(move |_| ending(panel, &keys, width, true)),
        confirming: panel.confirm_stop.is_some(),
    }
}

fn processes(panel: &Panel, columns: usize, room: usize) -> Vec<(Line, Part)> {
    let total = panel.processes.len();
    if total == 0 {
        let empty = Line::new("Nothing running", MUTED);
        return vec![(panel::indent(empty, columns), Part::Keep)];
    }
    let start = panel::start(panel.selected, total, room);
    let end = columns.saturating_sub(2);
    let labels = panel
        .processes
        .iter()
        .map(|process| text::cells(&display_status(process)))
        .max()
        .unwrap_or(0);
    let stopping = panel
        .confirm_stop
        .as_ref()
        .map(|process| process.id.as_str());
    let shown = panel
        .processes
        .iter()
        .enumerate()
        .skip(start)
        .take(room)
        .map(|(index, process)| {
            let on = index == panel.selected;
            let deleting = on && stopping == Some(process.id.as_str());
            let mut line = panel::marker(on, deleting);
            let label = display_status(process);
            line.push(&label, status_style(process));
            line.push(&" ".repeat(labels - text::cells(&label) + 2), MUTED);
            line.push(
                &single_line(&process.command),
                if deleting { BAD } else { STRONG },
            );
            let line = panel::spread(line, Line::new(&process.id, MUTED), end, None);
            panel::choice(line, on, columns)
        })
        .collect();
    panel::window(shown, start, total, room, columns)
        .into_iter()
        .enumerate()
        .map(|(offset, line)| {
            let index = start + offset;
            let part = if index == panel.selected {
                Part::Keep
            } else {
                Part::Row(index.abs_diff(panel.selected))
            };
            (line, part)
        })
        .collect()
}

/// The log rows of `text` wrapped at spaces to `width` cells.
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
        Line::new(text, BODY).at_source(0).wrap(width, true)
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

fn keys(process: Option<&Process>, logs: &Logs) -> Vec<Key<'static>> {
    if logs.input_mode {
        return vec![key("Ctrl+]", "back"), key("Ctrl+C", "interrupt")];
    }
    let other = STREAMS[1 - logs.stream];
    let mut keys = vec![key("↑↓", "scroll").scrolling(), key("Tab", other)];
    keys.push(if logs.views[logs.stream].follow {
        key("Home", "start").scrolling().optional()
    } else {
        key("End", "follow").scrolling()
    });
    keys.push(key("PgUp/PgDn", "page").scrolling().optional());
    if process.is_some_and(Process::active) {
        if logs.interactive {
            keys.push(key("i", "input"));
        }
        keys.push(key("Ctrl+D", "stop"));
    }
    keys.push(key("Esc", "back"));
    keys
}

/// The rows above the stream: failures, or why there is no output.
fn notes(panel: &Panel, logs: &Logs, columns: usize) -> Vec<Line> {
    let process = panel.selected_process();
    let view = &logs.views[logs.stream];
    let errors = [
        process.and_then(|process| process.error.as_deref()),
        view.error.as_deref(),
    ];
    let mut notes: Vec<Line> = errors
        .into_iter()
        .flatten()
        .map(|error| panel::indent(Line::new(&single_line(error), BAD), columns))
        .collect();
    let empty = rows(&stream(logs), text_width(columns)).is_empty();
    if empty && view.error.is_none() {
        let finished = process.is_none_or(|process| !process.active());
        let empty = if finished {
            "No output"
        } else {
            "No output yet"
        };
        notes.push(panel::indent(Line::new(empty, MUTED), columns));
    }
    notes
}

fn stream(logs: &Logs) -> String {
    logs.views[logs.stream].display_text(logs.interactive && logs.stream == 0)
}

/// Total rows of the open stream and the rows visible at once in a panel
/// body of `room` rows.
pub(in crate::tui) fn metrics(panel: &Panel, columns: usize, room: usize) -> (usize, usize) {
    let Some(logs) = &panel.logs else {
        return (0, room);
    };
    let notes = notes(panel, logs, columns).len();
    (
        rows(&stream(logs), text_width(columns)).len(),
        room.saturating_sub(notes).max(1),
    )
}

fn output<'a>(panel: &'a Panel, logs: &'a Logs, columns: usize) -> Sheet<'a> {
    let process = panel.selected_process();
    let view = &logs.views[logs.stream];
    let title = process.map_or_else(|| "Process".into(), |process| single_line(&process.command));
    let mut aside = match process {
        Some(process) if process.active() => Line::new("running", MUTED),
        Some(process) => Line::new(&display_status(process), status_style(process)),
        None => Line::new("unavailable", BAD),
    };
    aside.push(&format!(" · {}", logs.id), MUTED);
    // Keep the outcome visible beside long commands at ordinary widths.
    // The breadcrumb and the rule use twelve and eight cells respectively.
    let title = text::ellipsize(
        &title,
        columns.saturating_sub(panel::cells(&aside) + 20).max(12),
    );
    let mode = if logs.input_mode {
        Line::new("input", GOOD)
    } else if view.follow {
        Line::new("following", MUTED)
    } else {
        Line::new("paused", WARNING)
    };
    let total = notes(panel, logs, columns).len() + rows(&stream(logs), text_width(columns)).len();
    let width = panel::text_width(columns);
    Sheet {
        title: panel::title(Some("Processes"), &title, ""),
        aside,
        field: None,
        draft_caret: false,
        head: panel::tabs(&STREAMS, logs.stream, mode, columns).to_vec(),
        body: Box::new(move |room| log(panel, logs, columns, room)),
        total: Some(total),
        tail: Box::new(move |overflow| {
            ending(
                panel,
                &keys(panel.selected_process(), logs),
                width,
                overflow,
            )
        }),
        confirming: panel.confirm_stop.is_some(),
    }
}

/// The notes, then the stream's window: the newest rows while following,
/// else from the row holding the reading anchor.
fn log(panel: &Panel, logs: &Logs, columns: usize, room: usize) -> Vec<(Line, Part)> {
    let view = &logs.views[logs.stream];
    let mut out: Vec<(Line, Part)> = notes(panel, logs, columns)
        .into_iter()
        .map(|line| (line, Part::Keep))
        .collect();
    let limit = room.saturating_sub(out.len()).max(1);
    let all = rows(&stream(logs), text_width(columns));
    let total = all.len();
    let bottom = total.saturating_sub(limit);
    let offset = if view.follow {
        bottom
    } else {
        all.iter()
            .rposition(|line| {
                line.origin
                    .is_some_and(|origin| origin.character <= view.anchor)
            })
            .unwrap_or(0)
            .min(bottom)
    };
    let overflow = total > limit && columns >= 7;
    let inset = columns.saturating_sub(2 + usize::from(overflow)).max(1);
    for (index, line) in all.into_iter().enumerate().skip(offset).take(limit) {
        let mut row = Line::default();
        panel::push(&mut row, " ", MUTED, Some(CODE_SURFACE));
        let row = panel::surfaced(row.append(line), CODE_SURFACE);
        let mut row = panel::fit(row, inset, Some(CODE_SURFACE)).indent("  ", MUTED);
        if overflow {
            let (glyph, style) = panel::scrollbar(index - offset, offset, total, limit);
            row.push(glyph, style);
        }
        // The newest rows stay longest while following.
        let distance = if view.follow {
            offset + limit - 1 - index
        } else {
            index - offset
        };
        out.push((row, Part::Row(distance)));
    }
    out
}

fn status_style(process: &Process) -> Style {
    match (process.status, process.exit_code) {
        ("running" | "stopping", _) => GOOD,
        ("timed_out", _) => WARNING,
        ("exited", Some(0)) | ("stopped", _) => MUTED,
        _ => BAD,
    }
}

fn display_status(process: &Process) -> String {
    if process.active() {
        "running".into()
    } else {
        process.label()
    }
}

fn single_line(value: &str) -> String {
    text::clean(value).replace('\n', " ")
}

#[cfg(test)]
mod tests;
