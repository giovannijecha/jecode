//! Collaboration events as tool rows. A lone event is one row under its
//! AGENT N badge, a Work chapter member keeps one row, and Ctrl+O opens the
//! full receipt: instruction, workspace, changed files and report.

use super::{Card, Status, Tool, badge, field, hint, line_rows, rhythm};
use crate::{
    json::Value,
    tui::{
        line::Line,
        text,
        theme::{BAD, BODY, CODE_TEXT, DIM, GOOD, MUTED, STRONG, Style, WARNING},
    },
};

fn number(value: &Value, key: &str) -> usize {
    value.get(key).and_then(Value::as_usize).unwrap_or(0)
}

fn strings<'a>(value: &'a Value, key: &str) -> Vec<&'a str> {
    value
        .get(key)
        .and_then(Value::as_array)
        .map_or_else(Vec::new, |items| {
            items.iter().filter_map(Value::as_str).collect()
        })
}

fn records(event: &Value) -> &[Value] {
    event
        .get("records")
        .and_then(Value::as_array)
        .unwrap_or(&[])
}

/// The event's own collaborator as it was when the event happened.
fn record(event: &Value) -> Option<&Value> {
    let id = field(event, "id");
    records(event)
        .iter()
        .find(|record| field(record, "id") == id)
}

/// One line of model-written text, safe for the terminal.
fn inline(value: &str) -> String {
    text::clean(value)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn reviewed(record: &Value) -> bool {
    record.get("reviewed") == Some(&Value::Bool(true))
}

fn failed_check(record: &Value) -> Option<&str> {
    strings(record, "checks")
        .into_iter()
        .find(|check| check.to_ascii_lowercase().contains("fail"))
        .map(|check| check.split_once(": ").map_or(check, |(_, rest)| rest))
}

/// Failed checks and failed runs are `bad`; unknown outcomes `warning`.
/// Everything else is routine and keeps the family ink.
pub(super) fn status(event: &Value) -> Status {
    match field(event, "kind") {
        "unknown" => Status::Unknown,
        "returned"
            if !strings(event, "details").is_empty()
                || record(event).and_then(failed_check).is_some() =>
        {
            Status::Failed
        }
        _ => Status::Done,
    }
}

fn workspace(record: &Value) -> String {
    let mode = field(record, "workspace");
    format!(
        "{} workspace",
        if mode.is_empty() { "shared" } else { mode }
    )
}

/// Changed files with their `+` / `−` line counts, summed over the calls that
/// changed them. Sessions saved before counts existed list the path alone.
fn files(record: &Value) -> Vec<(String, Option<(usize, usize)>)> {
    strings(record, "files")
        .into_iter()
        .map(|path| {
            let counts = record
                .get("tools")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter(|tool| {
                    tool.get("arguments")
                        .is_some_and(|arguments| field(arguments, "path") == path)
                })
                .filter_map(|tool| tool.get("change"))
                .map(|change| (number(change, "added"), number(change, "removed")))
                .reduce(|left, right| (left.0 + right.0, left.1 + right.1));
            (path.to_owned(), counts)
        })
        .collect()
}

fn created(event: &Value) -> Vec<&Value> {
    let ids = strings(event, "details");
    records(event)
        .iter()
        .filter(|record| ids.contains(&field(record, "id")))
        .collect()
}

/// What happened, as the row's text. A member counts changed files; a lone
/// or open receipt names the assignment instead and lists the files below.
fn subject(event: &Value, member: bool) -> String {
    let version = number(event, "version");
    let title = inline(field(event, "title"));
    match field(event, "kind") {
        "created" => {
            let agents = created(event);
            let mut modes: Vec<_> = agents
                .iter()
                .map(|record| field(record, "workspace"))
                .collect();
            modes.sort_unstable();
            modes.dedup();
            let place = match modes.as_slice() {
                [mode] if agents.len() > 1 => format!("{mode} workspaces"),
                [mode] => format!("{mode} workspace"),
                _ => "shared and isolated workspaces".into(),
            };
            format!("{} created · {place}", strings(event, "details").len())
        }
        "sent" if version == 0 => "Stop requested".into(),
        "sent" => format!("Assignment v{version} sent"),
        "received" => format!("Assignment v{version} received"),
        "returned" if member => {
            let count = record(event).map_or(0, |record| strings(record, "files").len());
            if count == 0 {
                "Returned · no files changed".into()
            } else {
                format!("Returned · {} changed", text::counted(count, "file"))
            }
        }
        "returned" => format!("Returned · {title}"),
        "reviewed" => format!("Reviewed · report v{version}"),
        "stopped" => "Stopped".into(),
        "closed" => "Closed".into(),
        "unknown" => "Outcome unknown after restart".into(),
        other => inline(other),
    }
}

/// The outcome fact after a member's text, or on a lone receipt's tree.
fn fact(event: &Value) -> Option<(String, Style)> {
    match field(event, "kind") {
        "returned" => Some(if let Some(error) = strings(event, "details").first() {
            (format!("failed · {}", inline(error)), BAD)
        } else if let Some(check) = record(event).and_then(failed_check) {
            (format!("check failed · {}", inline(check)), BAD)
        } else if record(event).is_some_and(reviewed) {
            ("reviewed by main agent".into(), MUTED)
        } else {
            ("pending review by main agent".into(), MUTED)
        }),
        "unknown" => Some(("inspect in /agents".into(), WARNING)),
        _ => None,
    }
}

/// One member row inside a Work chapter: the name in its family ink, what
/// happened and its fact.
pub(super) fn member(tool: &Tool<'_>, columns: usize) -> Vec<Line> {
    let Some(event) = tool.result else {
        return Vec::new();
    };
    let name = field(tool.arguments, "label");
    let mut line = Line::new(&name.to_uppercase(), badge::ink(name).bold());
    line.push(&format!(" {}", subject(event, true)), BODY);
    if let Some((fact, style)) = fact(event) {
        let suffix = format!("  {fact}");
        line = line.shortened(columns.saturating_sub(4 + text::cells(&suffix)));
        line.push(&suffix, style);
    }
    vec![rhythm::one(line, columns, "  ⎿ ", DIM, 2)]
}

fn file_row(path: &str, counts: Option<(usize, usize)>) -> Line {
    let mut line = Line::new(&inline(path), CODE_TEXT);
    if let Some((added, removed)) = counts {
        line.push(&format!("  +{added}"), GOOD);
        line.push(&format!(" −{removed}"), BAD);
    }
    line
}

/// A lone receipt's tree while collapsed: changed files and the outcome.
pub(super) fn receipts(tool: &Tool<'_>, columns: usize) -> Vec<Line> {
    const FILES: usize = 4;
    let Some(event) = tool.result else {
        return Vec::new();
    };
    let mut rows = Vec::new();
    if field(event, "kind") == "returned" {
        let files = record(event).map(files).unwrap_or_default();
        for (path, counts) in files.iter().take(FILES) {
            rows.push(file_row(path, *counts));
        }
        if files.len() > FILES {
            rows.push(Line::new(
                &format!("+{} more", text::counted(files.len() - FILES, "file")),
                DIM,
            ));
        }
    }
    if let Some((fact, style)) = fact(event) {
        rows.push(Line::new(&fact, style));
    }
    rows.into_iter()
        .enumerate()
        .map(|(index, line)| rhythm::one(line, columns, "  ⎿ ", DIM, index + 1))
        .collect()
}

/// The full receipt that Ctrl+O opens.
fn details(event: &Value, columns: usize) -> Vec<Line> {
    let mut lines = Vec::new();
    let record = record(event);
    let assignment = |record: &Value| {
        Line::new(
            &format!("{} · {}", inline(field(record, "title")), workspace(record)),
            DIM,
        )
    };
    let version = number(event, "version");
    match field(event, "kind") {
        "created" => {
            for record in created(event) {
                let mut head = Line::new(
                    &format!("AGENT {}", number(record, "number")),
                    badge::ink("Agent").bold(),
                );
                head.push(&format!(" {}", inline(field(record, "title"))), STRONG);
                head.push(&format!("  {}", workspace(record)), DIM);
                lines.push(head);
                let instruction = if number(record, "received_version") == 0 {
                    field(record, "pending_instruction")
                } else {
                    field(record, "instructions")
                };
                if !instruction.is_empty() {
                    lines.push(Line::new(&text::clean(instruction), MUTED));
                }
            }
        }
        "sent" => {
            if let Some(record) = record {
                let instruction = if version > 0 && number(record, "received_version") < version {
                    field(record, "pending_instruction")
                } else if version > 0 {
                    field(record, "instructions")
                } else {
                    ""
                };
                if !instruction.is_empty() {
                    lines.push(Line::new(&text::clean(instruction), MUTED));
                }
                lines.push(assignment(record));
            }
        }
        "returned" => {
            if let Some(record) = record {
                for (path, counts) in files(record) {
                    lines.push(file_row(&path, counts));
                }
                if let Some(report) = record
                    .get("report")
                    .filter(|report| **report != Value::Null)
                {
                    let report = field(report, "text");
                    if !report.trim().is_empty() {
                        lines.push(Line::new(&text::clean(report.trim()), MUTED));
                    }
                }
            }
            for error in strings(event, "details") {
                lines.push(Line::new(&text::clean(error), BAD));
            }
            if let Some(check) = record.and_then(failed_check) {
                lines.push(Line::new(&format!("Check failed · {}", inline(check)), BAD));
            }
            lines.push(Line::new(
                if record.is_some_and(reviewed) {
                    "Reviewed by main agent"
                } else {
                    "Pending review by main agent"
                },
                MUTED,
            ));
        }
        "reviewed" => {
            for note in strings(event, "details") {
                lines.push(Line::new(&text::clean(note), MUTED));
            }
        }
        "unknown" => {
            for message in strings(event, "details") {
                lines.push(Line::new(&text::clean(message), MUTED));
            }
            lines.push(Line::new("Inspect in /agents", WARNING));
        }
        "stopped" | "closed" => lines.extend(record.map(assignment)),
        _ => {}
    }
    let mut source = 1;
    let mut rows = Vec::new();
    for line in lines {
        let wrapped = line_rows(&line, columns, source, "  ⎿ ", "    ", DIM);
        source += wrapped.len();
        rows.extend(wrapped);
    }
    rows
}

/// The lone row, or the open receipt when expanded.
pub(super) fn card(tool: Tool<'_>, columns: usize, keycap: bool) -> Card {
    let Some(event) = tool.result else {
        return Card {
            rows: Vec::new(),
            hidden: false,
        };
    };
    let name = field(tool.arguments, "label");
    let mut title = badge::badge(name, status(event));
    title.push(&format!(" {}", subject(event, false)), BODY);
    let title = title.at_source(0).shortened(columns);
    let expanded = tool.presentation.expanded;
    let more = details(event, columns);
    let hidden = !expanded && !more.is_empty();
    let mut rows = if keycap && hidden {
        hint(title, "expand", columns)
    } else {
        vec![title]
    };
    let titled = rows.len();
    rows.extend(if expanded {
        more
    } else {
        receipts(&tool, columns)
    });
    badge::hang(&mut rows[titled..], true);
    Card { rows, hidden }
}
