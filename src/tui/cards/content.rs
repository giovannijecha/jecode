use super::{
    Body, Status, Tool, diff, discovery, edit_fallback, field, flag, line_rows, preview, streams,
    summary::size,
};
use crate::{
    file_change::Change,
    json::Value,
    tui::{
        line::{Fold, Line},
        syntax::Highlighter,
        text,
        theme::{CODE_TEXT, DIM, MUTED, Style, WARNING},
    },
};

pub(super) fn payload(value: &str) -> Vec<String> {
    if value.is_empty() {
        return Vec::new();
    }
    let value = text::clean(value);
    value
        .strip_suffix('\n')
        .unwrap_or(&value)
        .split('\n')
        .map(str::to_owned)
        .collect()
}

pub(super) fn render(
    tool: &Tool<'_>,
    result: &Value,
    status: Status,
    columns: usize,
    expanded: bool,
) -> Body {
    let arguments = tool.arguments;
    match tool.name {
        "list" | "search" => discovery::render(tool.name, result, columns, expanded),
        "read" if !expanded => Body::new(Vec::new()),
        "read" if result.get("content").and_then(Value::as_str).is_some() => {
            Body::new(read(arguments, result, columns))
        }
        "read" if result.get("attachment").is_some() || result.get("view").is_some() => {
            Body::new(Vec::new())
        }
        "write" => written(arguments, columns, expanded),
        "edit" => edit_fallback::body(arguments, result, columns, expanded)
            .unwrap_or_else(|| Body::new(Vec::new())),
        _ if result.get("processes").is_some() => processes(result, columns, expanded),
        "bash" | "exec" | "process" => {
            let failed = status == Status::Failed;
            let skip = (status != Status::Stopped)
                .then(|| streams::headline(&streams::entries(result, failed), failed))
                .flatten();
            streams::render(result, failed, skip, columns, expanded)
        }
        "checkpoint" => {
            let lines = payload(&crate::checkpoints::render::text(result, "/checkpoints"));
            selected(&lines, columns, expanded, 0)
        }
        // A page's text, or the raw result, waits for Ctrl+O.
        "web" if result.get("preview").and_then(Value::as_str).is_some() => {
            let lines: Vec<_> = payload(field(result, "preview"))
                .into_iter()
                .filter(|line| !line.trim().is_empty())
                .collect();
            selected(&lines, columns, expanded, 0)
        }
        _ => selected(&payload(&result.pretty()), columns, expanded, 0),
    }
}

fn read(arguments: &Value, result: &Value, columns: usize) -> Vec<Line> {
    let content = payload(field(result, "content"));
    let numbered =
        arguments.get("byte_offset").is_none() && result.get("offset") != Some(&Value::Null);
    let entries: Vec<_> =
        content
            .iter()
            .map(|line| {
                let parsed = numbered.then(|| line.split_once(": ")).flatten().and_then(
                    |(number, source)| number.parse::<usize>().ok().map(|number| (number, source)),
                );
                parsed.map_or((None, line.as_str()), |(number, source)| {
                    (Some(number), source)
                })
            })
            .collect();
    let digits = entries
        .iter()
        .filter_map(|(number, _)| *number)
        .max()
        .map_or(0, |number| number.to_string().len());
    let mut lexer = Highlighter::for_path(field(arguments, "path"));
    let fold = Fold {
        markup: lexer.markup(),
        ..Fold::default()
    };
    entries
        .into_iter()
        .enumerate()
        .flat_map(|(index, (number, source))| {
            let label = if digits == 0 {
                String::new()
            } else {
                number.map_or_else(
                    || " ".repeat(digits + 3),
                    |number| format!("{number:>digits$}   "),
                )
            };
            diff::code_rows(
                lexer.line(source),
                &Line::new(&label, DIM),
                "    ",
                columns,
                index + 2,
                fold,
            )
        })
        .collect()
}

fn written(arguments: &Value, columns: usize, expanded: bool) -> Body {
    source(
        &payload(field(arguments, "content")),
        field(arguments, "path"),
        columns,
        expanded,
    )
}

/// A new file is captured source, with the same number gutter as an open Read.
pub(super) fn created(change: &Change, path: &str, columns: usize, expanded: bool) -> Body {
    let lines: Vec<_> = change
        .hunks
        .iter()
        .flatten()
        .map(|entry| text::clean(&entry.text))
        .collect();
    source(&lines, path, columns, expanded)
}

fn source(lines: &[String], path: &str, columns: usize, expanded: bool) -> Body {
    let count = if expanded {
        lines.len()
    } else {
        lines.len().min(6)
    };
    let digits = count.max(1).to_string().len();
    let mut lexer = Highlighter::for_path(path);
    // Collapsed, each line keeps at most two rows.
    let fold = Fold {
        rows: (!expanded).then_some(2),
        markup: lexer.markup(),
        ..Fold::default()
    };
    let rows = lines
        .iter()
        .take(count)
        .enumerate()
        .flat_map(|(index, line)| {
            diff::code_rows(
                lexer.line(line),
                &Line::new(&format!("{:>digits$}   ", index + 1), DIM),
                "    ",
                columns,
                index + 2,
                fold,
            )
        })
        .collect();
    Body {
        rows,
        hidden: lines.len() - count,
        hidden_hunks: 0,
        unit: "lines",
    }
}

fn processes(result: &Value, columns: usize, expanded: bool) -> Body {
    let lines: Vec<_> = result
        .get("processes")
        .and_then(Value::as_array)
        .unwrap_or(&[])
        .iter()
        .map(|process| {
            let exit = process
                .get("exit_code")
                .filter(|value| **value != Value::Null)
                .map_or_else(String::new, |code| format!(" ({})", code.encode()));
            format!(
                "{}  {}{exit}  {}",
                field(process, "process_id"),
                field(process, "status"),
                field(process, "command")
            )
        })
        .collect();
    selected(&lines, columns, expanded, 0)
}

/// The first `preview_rows` lines, or every line when expanded.
fn selected(lines: &[String], columns: usize, expanded: bool, preview_rows: usize) -> Body {
    let count = if expanded {
        lines.len()
    } else {
        lines.len().min(preview_rows)
    };
    let rows = lines
        .iter()
        .take(count)
        .enumerate()
        .flat_map(|(index, line)| {
            line_rows(
                &preview(
                    Line::new(line, CODE_TEXT),
                    columns.saturating_sub(4),
                    expanded,
                ),
                columns,
                index + 2,
                "    ",
                "    ",
                MUTED,
            )
        })
        .collect();
    Body {
        rows,
        hidden: lines.len() - count,
        hidden_hunks: 0,
        unit: "lines",
    }
}

/// Upright notes under the card: what was trimmed, what follows, what failed.
pub(super) fn notices(name: &str, result: &Value, columns: usize, expanded: bool) -> Vec<Line> {
    let mut notices: Vec<(Style, String)> = Vec::new();
    let number = |key: &str| result.get(key).and_then(Value::as_usize).unwrap_or(0);
    if name == "read"
        && expanded
        && (result.get("attachment").is_some() || result.get("view").is_some())
        && let Some(note) = result.get("note").and_then(Value::as_str)
    {
        notices.push((MUTED, note.into()));
    }
    if matches!(name, "bash" | "exec" | "process") {
        for stream in ["stdout", "stderr"] {
            let skipped = number(&format!("{stream}_skipped_bytes"));
            if skipped > 0 {
                notices.push((
                    MUTED,
                    format!(
                        "{stream} skipped the first {} · full output saved",
                        size(skipped)
                    ),
                ));
            }
            if flag(result, &format!("{stream}_truncated")) {
                let bytes = size(number(&format!("{stream}_bytes")));
                let limit = size(number("output_limit_bytes"));
                notices.push(if field(result, &format!("{stream}_file")).is_empty() {
                    (WARNING, format!("{stream} capture kept {limit} of {bytes}"))
                } else {
                    (
                        MUTED,
                        format!(
                            "{stream} trimmed to the last {limit} of {bytes} · full output saved"
                        ),
                    )
                });
            }
        }
    }
    if let Some(checkpoint) = result.get("checkpoint") {
        let text = |key| checkpoint.get(key).and_then(Value::as_str);
        if let Some(status @ ("partial" | "none")) = text("status")
            && (expanded || status == "partial" || matches!(name, "read" | "write" | "edit"))
        {
            notices.push((
                WARNING,
                text("reason").map_or_else(
                    || format!("Checkpoint {status}"),
                    |reason| format!("Checkpoint {status} · {reason}"),
                ),
            ));
        }
        if expanded && let Some(reference) = text("ref") {
            notices.push((MUTED, format!("Checkpoint {reference}")));
        }
    }
    notices
        .into_iter()
        .enumerate()
        .flat_map(|(index, (style, message))| {
            line_rows(
                &Line::new(&text::clean(&message), style),
                columns,
                usize::MAX - 100 + index,
                "    ",
                "    ",
                MUTED,
            )
        })
        .collect()
}
