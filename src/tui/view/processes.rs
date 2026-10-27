//! The `/terminal` panel in the menu standard: the process list is a menu,
//! stopping asks in place of the key row, and output sits in an inset with
//! Output and Errors as section tabs.

use super::panel::{self, Key, Panel as Rows, Part, key};
use crate::tools::processes::Process;
use crate::tui::{
    line::Line,
    processes::{Logs, Panel, STREAMS},
    text,
    theme::{
        ACCENT, BAD, BODY, CODE_SURFACE, GOOD, MUTED, SELECTED_SURFACE, STRONG, Style, WARNING,
    },
};

const LIMIT: usize = 6;

pub(super) fn render(
    panel: &Panel,
    columns: usize,
    height: usize,
) -> (Vec<Line>, Option<(usize, usize)>) {
    let width = panel::width(columns);
    let out = if let Some(logs) = &panel.logs {
        output(panel, logs, width, height)
    } else {
        list(panel, width, height)
    };
    out.finish(columns, height)
}

/// Process rows visible at once.
pub(in crate::tui) fn list_capacity(height: usize) -> usize {
    panel::budget(height, 2, LIMIT)
}

/// Log text width inside the inset, leaving room for the scrollbar.
pub(in crate::tui) fn text_width(width: usize) -> usize {
    width.saturating_sub(4).max(1)
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

fn list(panel: &Panel, width: usize, height: usize) -> Rows {
    let total = panel.processes.len();
    let mut out = Rows::default();
    let aside = if total == 0 {
        String::new()
    } else {
        format!("{total} running")
    };
    out.add(
        panel::header(
            None,
            "Background processes",
            "this conversation",
            Line::new(&aside, MUTED),
            width,
        ),
        Part::Title,
    );
    out.add(panel::divider(width), Part::Header);
    out.space();
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
    let limit = list_capacity(height);
    let overflow = total > limit;
    let ending = ending(panel, &keys, width, overflow);
    if total == 0 {
        out.add(
            Line::new("No active background processes in this conversation", MUTED),
            Part::Keep,
        );
    }
    let start = panel
        .selected
        .saturating_sub(limit / 2)
        .min(total.saturating_sub(limit));
    let rows = width.saturating_sub(2 * usize::from(overflow)).max(1);
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
    for (index, process) in panel.processes.iter().enumerate().skip(start).take(limit) {
        let on = index == panel.selected;
        let mut line = Line::new(if on { "› " } else { "  " }, ACCENT);
        let label = display_status(process);
        line.push(&label, status_style(process));
        line.push(&" ".repeat(labels - text::cells(&label) + 2), MUTED);
        let style = if on && stopping == Some(process.id.as_str()) {
            BAD
        } else {
            STRONG
        };
        line.push(&single_line(&process.command), style);
        let line = panel::spread(line, Line::new(&process.id, MUTED), rows, None);
        let line = if on {
            panel::surfaced(
                panel::fit(line, rows, Some(SELECTED_SURFACE)),
                SELECTED_SURFACE,
            )
        } else {
            line
        };
        let bar = overflow.then_some((index - start, start, total, limit));
        let part = if on {
            Part::Keep
        } else {
            Part::Row(index.abs_diff(panel.selected))
        };
        out.add(panel::barred(line, bar), part);
    }
    out.space();
    out.extend(ending, confirming(panel));
    out
}

/// A stop confirmation outlasts every other row.
fn confirming(panel: &Panel) -> Part {
    if panel.confirm_stop.is_some() {
        Part::Focus
    } else {
        Part::Keep
    }
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

/// Rows of the stream that fit beside the header, tabs, notes and keys.
fn capacity(panel: &Panel, logs: &Logs, width: usize, height: usize) -> usize {
    let process = panel.selected_process();
    let notes = usize::from(process.and_then(|process| process.error.as_ref()).is_some())
        + usize::from(logs.views[logs.stream].error.is_some());
    let keys = ending(panel, &keys(process, logs), width, true).len();
    let chrome = 3 + 4 + notes + keys;
    if height >= chrome + 3 {
        height - chrome
    } else {
        height.saturating_sub(1 + notes + keys)
    }
    .max(1)
}

/// Total rows of the open stream and the rows visible at once.
pub(in crate::tui) fn metrics(panel: &Panel, width: usize, height: usize) -> (usize, usize) {
    let Some(logs) = &panel.logs else {
        return (0, list_capacity(height));
    };
    (
        rows(
            &logs.views[logs.stream].display_text(logs.interactive && logs.stream == 0),
            text_width(width),
        )
        .len(),
        capacity(panel, logs, width, height),
    )
}

fn output(panel: &Panel, logs: &Logs, width: usize, height: usize) -> Rows {
    let process = panel.selected_process();
    let view = &logs.views[logs.stream];
    let limit = capacity(panel, logs, width, height);
    let all = rows(
        &view.display_text(logs.interactive && logs.stream == 0),
        text_width(width),
    );
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
    let overflow = total > limit;
    let title = process.map_or_else(|| "Process".into(), |process| single_line(&process.command));
    let mut aside = match process {
        Some(process) if process.active() => {
            let mut line = Line::new("● ", GOOD);
            line.push("running", MUTED);
            line
        }
        Some(process) => Line::new(&display_status(process), status_style(process)),
        None => Line::new("unavailable", BAD),
    };
    aside.push(&format!(" · {}", logs.id), MUTED);
    let mut out = Rows::default();
    out.add(
        panel::header(Some("Processes"), &title, "", aside, width),
        Part::Title,
    );
    let mode = if logs.input_mode {
        Line::new("input", GOOD)
    } else if view.follow {
        Line::new("following", MUTED)
    } else {
        Line::new("paused", WARNING)
    };
    out.extend(
        panel::tabs(&STREAMS, logs.stream, mode, width),
        Part::Header,
    );
    out.space();
    let errors = [
        process.and_then(|process| process.error.as_deref()),
        view.error.as_deref(),
    ];
    for error in errors.into_iter().flatten() {
        out.add(
            Line::new(&single_line(error), BAD).shortened(width),
            Part::Keep,
        );
    }
    if all.is_empty() && view.error.is_none() {
        let finished = process.is_none_or(|process| !process.active());
        let empty = if finished {
            "No output"
        } else {
            "No output yet"
        };
        out.add(Line::new(empty, MUTED), Part::Keep);
    }
    let inset = width.saturating_sub(2 * usize::from(overflow)).max(1);
    for (index, line) in all.into_iter().enumerate().skip(offset).take(limit) {
        let mut row = Line::default();
        panel::push(&mut row, " ", MUTED, Some(CODE_SURFACE));
        let row = panel::surfaced(row.append(line), CODE_SURFACE);
        let row = panel::fit(row, inset, Some(CODE_SURFACE));
        let bar = overflow.then_some((index - offset, offset, total, limit));
        // The newest rows stay longest while following.
        let distance = if view.follow {
            offset + limit - 1 - index
        } else {
            index - offset
        };
        out.add(panel::barred(row, bar), Part::Row(distance));
    }
    out.space();
    out.extend(
        ending(panel, &keys(process, logs), width, overflow),
        confirming(panel),
    );
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
