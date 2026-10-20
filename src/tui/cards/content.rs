use super::{Body, diff, field, flag, preview, wrapped};
use crate::{
    json::Value,
    tui::{
        line::Line,
        notice::{self, Level},
        syntax::Highlighter,
        text,
        theme::{BAD, BODY, MUTED},
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
    name: &str,
    arguments: &Value,
    result: &Value,
    columns: usize,
    failed: bool,
    expanded: bool,
) -> Body {
    match name {
        "read" if !expanded => Body::new(Vec::new()),
        "read" if result.get("content").and_then(Value::as_str).is_some() => {
            Body::new(read(arguments, result, columns))
        }
        "write" => written(arguments, columns, expanded),
        "edit" => selected(&payload(field(arguments, "new_text")), columns, expanded, 3),
        _ if result.get("processes").is_some() => processes(result, columns, expanded),
        "bash" | "process" => streams(result, columns, failed, expanded),
        _ => {
            let lines = payload(&result.pretty());
            selected(&lines, columns, expanded, 3)
        }
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
            diff::code_rows(lexer.line(source), &label, MUTED, None, columns, index + 2)
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
                None,
                columns,
                index + 2,
            )
        })
        .collect();
    Body {
        rows,
        hidden: lines.len() - count,
        unit: "output lines",
    }
}

fn streams(result: &Value, columns: usize, failed: bool, expanded: bool) -> Body {
    let stdout = payload(field(result, "stdout"));
    let stderr = payload(field(result, "stderr"));
    let stderr = stderr
        .iter()
        .map(|line| (line, if failed { BAD } else { MUTED }, "stderr"));
    let stdout = stdout.iter().map(|line| (line, BODY, "stdout"));
    let entries: Vec<_> = if failed {
        stderr.chain(stdout).collect()
    } else {
        stdout.chain(stderr).collect()
    };
    let limit = if failed { 6 } else { 3 };
    let start = if !expanded && !failed {
        entries.len().saturating_sub(limit)
    } else {
        0
    };
    let count = if expanded {
        entries.len()
    } else {
        entries.len().min(limit)
    };
    let both = !field(result, "stdout").is_empty() && !field(result, "stderr").is_empty();
    let mut previous = "";
    let mut rows = Vec::new();
    for (index, (line, style, stream)) in entries.iter().enumerate().skip(start).take(count) {
        if expanded && both && previous != *stream {
            rows.extend(wrapped(
                stream,
                MUTED,
                columns,
                index * 2 + 2,
                "    ",
                "    ",
                true,
            ));
        }
        previous = stream;
        rows.extend(wrapped(
            &preview(Line::new(line, *style), columns.saturating_sub(4), expanded).plain(),
            *style,
            columns,
            index * 2 + 3,
            "    ",
            "    ",
            false,
        ));
    }
    Body {
        rows,
        hidden: entries.len() - count,
        unit: "output lines",
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

fn selected(lines: &[String], columns: usize, expanded: bool, limit: usize) -> Body {
    let count = if expanded {
        lines.len()
    } else {
        lines.len().min(limit)
    };
    let rows = lines
        .iter()
        .take(count)
        .enumerate()
        .flat_map(|(index, line)| {
            wrapped(
                &preview(Line::new(line, BODY), columns.saturating_sub(4), expanded).plain(),
                BODY,
                columns,
                index + 2,
                "    ",
                "    ",
                false,
            )
        })
        .collect();
    Body {
        rows,
        hidden: lines.len() - count,
        unit: "output lines",
    }
}

pub(super) fn notices(name: &str, result: &Value, columns: usize) -> Vec<Line> {
    let mut notices = Vec::new();
    if name == "read" && result.get("offset") == Some(&Value::Null) {
        if let Some(next) = result.get("next_byte_offset").and_then(Value::as_usize) {
            notices.push((
                Level::Info,
                format!("More file bytes available from byte {next}."),
            ));
        }
    } else if name == "read"
        && let Some(next) = result.get("next_offset").and_then(Value::as_usize)
    {
        notices.push((
            Level::Info,
            if flag(result, "partial_line") {
                format!(
                    "Line continues at byte {}.",
                    result
                        .get("next_byte_offset")
                        .and_then(Value::as_usize)
                        .unwrap_or(0)
                )
            } else {
                format!("More file lines available from line {next}.")
            },
        ));
    }
    if matches!(name, "bash" | "process") {
        for stream in ["stdout", "stderr"] {
            let skipped = result
                .get(&format!("{stream}_skipped_bytes"))
                .and_then(Value::as_usize)
                .unwrap_or(0);
            if skipped > 0 {
                notices.push((
                    Level::Warning,
                    format!("{stream}: {skipped} earlier bytes not shown · full output saved."),
                ));
            }
            if flag(result, &format!("{stream}_truncated")) {
                let bytes = result
                    .get(&format!("{stream}_bytes"))
                    .and_then(Value::as_usize)
                    .unwrap_or(0);
                let limit = result
                    .get("output_limit_bytes")
                    .and_then(Value::as_usize)
                    .unwrap_or(0);
                notices.push((
                    Level::Warning,
                    if field(result, &format!("{stream}_file")).is_empty() {
                        format!("{stream} capture truncated: {limit} of {bytes} bytes retained.")
                    } else {
                        format!(
                            "{stream}: last {limit} of {bytes} bytes shown · full output saved."
                        )
                    },
                ));
            }
        }
    }
    notices
        .into_iter()
        .enumerate()
        .flat_map(|(index, (level, message))| {
            notice::lines(level, &message, columns.saturating_sub(4).max(1))
                .into_iter()
                .map(move |row| {
                    row.at_source(usize::MAX - 100 + index)
                        .gutter("    ", MUTED)
                        .shortened(columns)
                })
        })
        .collect()
}
