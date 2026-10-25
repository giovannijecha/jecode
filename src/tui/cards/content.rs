use super::{
    Body, Status, Tool, diff, edit_fallback, field, flag, line_rows, preview,
    streams::{self, Preview},
    summary::size,
};
use crate::{
    json::Value,
    tui::{
        line::Line,
        syntax::Highlighter,
        text,
        theme::{BODY, MUTED, Style, WARNING},
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
        "read" if !expanded => Body::new(Vec::new()),
        "read" if result.get("content").and_then(Value::as_str).is_some() => {
            Body::new(read(arguments, result, columns))
        }
        "write" => written(arguments, columns, expanded),
        "edit" => edit_fallback::body(arguments, result, columns, expanded)
            .unwrap_or_else(|| Body::new(Vec::new())),
        _ if result.get("processes").is_some() => processes(result, columns, expanded),
        "bash" | "process" => {
            let (skip, rows) = match status {
                Status::Failed => (
                    streams::headline(&streams::entries(result, true), true),
                    Preview::First(2),
                ),
                Status::Stopped => (None, Preview::Last(3)),
                _ => (
                    streams::headline(&streams::entries(result, false), false),
                    Preview::Quiet,
                ),
            };
            let failed = status == Status::Failed;
            streams::render(result, failed, skip, rows, columns, expanded)
        }
        "checkpoint" => {
            let lines = payload(&crate::checkpoints::render::text(result, "/checkpoints"));
            selected(&lines, columns, expanded, 3)
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
            diff::code_rows(lexer.line(source), &label, MUTED, columns, index + 2)
        })
        .collect()
}

fn written(arguments: &Value, columns: usize, expanded: bool) -> Body {
    let lines = payload(field(arguments, "content"));
    let count = if expanded {
        lines.len()
    } else {
        lines.len().min(14)
    };
    let digits = lines.len().max(1).to_string().len();
    let mut lexer = Highlighter::for_path(field(arguments, "path"));
    let rows = lines
        .iter()
        .take(count)
        .enumerate()
        .flat_map(|(index, source)| {
            diff::code_rows(
                preview(
                    lexer.line(source),
                    columns.saturating_sub(7 + digits),
                    expanded,
                ),
                &format!("{:>digits$}   ", index + 1),
                MUTED,
                columns,
                index + 2,
            )
        })
        .collect();
    Body {
        rows,
        hidden: lines.len() - count,
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
    selected(&lines, columns, expanded, 3)
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
                &preview(Line::new(line, BODY), columns.saturating_sub(4), expanded),
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
        unit: "lines",
    }
}

/// Upright notes under the card: what was trimmed, what follows, what failed.
pub(super) fn notices(name: &str, result: &Value, columns: usize, expanded: bool) -> Vec<Line> {
    let mut notices: Vec<(Style, String)> = Vec::new();
    let number = |key: &str| result.get(key).and_then(Value::as_usize).unwrap_or(0);
    if name == "read" && result.get("offset") == Some(&Value::Null) {
        if let Some(next) = result.get("next_byte_offset").and_then(Value::as_usize) {
            notices.push((MUTED, format!("More bytes available from byte {next}")));
        }
    } else if name == "read"
        && let Some(next) = result.get("next_offset").and_then(Value::as_usize)
    {
        notices.push((
            MUTED,
            if flag(result, "partial_line") {
                format!("Line continues at byte {}", number("next_byte_offset"))
            } else {
                format!("More lines available from line {next}")
            },
        ));
    }
    if matches!(name, "bash" | "process") {
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
        if let Some(status @ ("partial" | "none")) = text("status") {
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
