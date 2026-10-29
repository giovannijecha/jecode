//! Structured map receipts. Notes and source freshness remain readable without raw JSON.

use super::{Body, line_rows};
use crate::{
    json::Value,
    tui::{
        line::Line,
        text,
        theme::{BAD, CODE_TEXT, DIM, MUTED, STRONG, Style, WARNING},
    },
};

pub(super) fn render(result: &Value, columns: usize, expanded: bool) -> Body {
    if !expanded {
        return Body::new(Vec::new());
    }
    let mut rows = Vec::new();
    match field(result, "action") {
        "remember" => {
            if let Some(note) = result.get("note") {
                entry(&mut rows, note, columns);
            } else {
                push(&mut rows, "Note saved", columns, DIM);
            }
        }
        "forget" => {
            let message = if result.get("removed") == Some(&Value::Bool(true)) {
                "Note removed"
            } else {
                "Note not found"
            };
            push(&mut rows, message, columns, DIM);
        }
        _ => {
            for item in array(result, "entries") {
                entry(&mut rows, item, columns);
            }
            if rows.is_empty() {
                push(&mut rows, "No map entries", columns, DIM);
            }
            let skipped = result.get("skipped");
            let skipped: Vec<_> = ["hidden", "ignored", "links", "binary", "non_utf8", "other"]
                .into_iter()
                .filter_map(|key| {
                    let count = skipped
                        .and_then(|item| item.get(key))
                        .and_then(Value::as_usize)?;
                    (count > 0).then(|| format!("{key} {count}"))
                })
                .collect();
            if !skipped.is_empty() {
                push(
                    &mut rows,
                    &format!("Skipped: {}", skipped.join(", ")),
                    columns,
                    DIM,
                );
            }
            for error in array(result, "errors") {
                push(
                    &mut rows,
                    &format!(
                        "Error {}: {}",
                        clean(field(error, "path"), 160),
                        clean(field(error, "error"), 240)
                    ),
                    columns,
                    BAD,
                );
            }
            let shown = array(result, "errors").len();
            let total = result
                .get("errors_count")
                .and_then(Value::as_usize)
                .unwrap_or(shown);
            if total > shown {
                push(
                    &mut rows,
                    &format!("{} more errors not shown", total - shown),
                    columns,
                    BAD,
                );
            }
            if result.get("complete") == Some(&Value::Bool(false))
                || result.get("inventory_truncated") == Some(&Value::Bool(true))
            {
                let notice = result
                    .get("next_offset")
                    .and_then(Value::as_usize)
                    .map_or_else(|| "Partial map".into(), |next| format!("More from {next}"));
                push(&mut rows, &notice, columns, WARNING);
            }
        }
    }
    Body::new(rows)
}

fn entry(rows: &mut Vec<Line>, item: &Value, columns: usize) {
    let kind = field(item, "kind");
    let title = field(item, "title");
    let path = field(item, "path");
    let label = if title.is_empty() { path } else { title };
    let mut head = Line::new(
        &format!(
            "{}  {}",
            if kind.is_empty() { "item" } else { kind },
            clean(label, 240)
        ),
        STRONG,
    );
    if !path.is_empty() && path != label {
        head.push(&format!("  {}", clean(path, 240)), CODE_TEXT);
    }
    push_line(rows, head, columns);
    let role = field(item, "role");
    if !role.is_empty() {
        push(rows, &clean(role, 320), columns, MUTED);
    }
    let summary = field(item, "summary");
    if !summary.is_empty() {
        push(rows, &clean(summary, 500), columns, MUTED);
    }
    let freshness = field(item, "freshness");
    let origin = field(item, "origin");
    let id = field(item, "id");
    let file_count = item.get("file_count").and_then(Value::as_usize);
    let mut facts = Vec::new();
    if !origin.is_empty() {
        facts.push(origin.to_owned());
    }
    if !freshness.is_empty() {
        facts.push(freshness.to_owned());
    }
    if !id.is_empty() {
        facts.push(format!("id {id}"));
    }
    if let Some(count) = file_count {
        facts.push(format!("{count} files"));
    }
    if !facts.is_empty() {
        let style = if matches!(freshness, "changed" | "missing" | "unverified") {
            WARNING
        } else {
            DIM
        };
        push(rows, &clean(&facts.join(" · "), 320), columns, style);
    }
    for source in array(item, "sources") {
        let source_path = field(source, "path");
        let line = source
            .get("line")
            .and_then(Value::as_usize)
            .map_or_else(String::new, |number| format!(":{number}"));
        let status = field(source, "status");
        let text = field(source, "text");
        let error = field(source, "error");
        let mut reference = format!("  {}{}", clean(source_path, 200), line);
        if !status.is_empty() {
            reference.push_str(&format!("  [{status}]"));
        }
        if !text.is_empty() {
            reference.push_str(&format!("  {}", clean(text, 320)));
        }
        let style = if matches!(status, "changed" | "missing" | "unverified") {
            WARNING
        } else {
            CODE_TEXT
        };
        push(rows, &reference, columns, style);
        if !error.is_empty() {
            push(
                rows,
                &format!("    {}", clean(error, 320)),
                columns,
                WARNING,
            );
        }
    }
}

fn push(rows: &mut Vec<Line>, value: &str, columns: usize, style: Style) {
    push_line(rows, Line::new(value, style), columns);
}

fn push_line(rows: &mut Vec<Line>, line: Line, columns: usize) {
    let source = rows.len() + 2;
    rows.extend(line_rows(&line, columns, source, "    ", "    ", DIM));
}

fn array<'a>(value: &'a Value, key: &str) -> &'a [Value] {
    value.get(key).and_then(Value::as_array).unwrap_or(&[])
}

fn field<'a>(value: &'a Value, key: &str) -> &'a str {
    value.get(key).and_then(Value::as_str).unwrap_or("")
}

fn clean(value: &str, max_cells: usize) -> String {
    text::ellipsize(&text::clean(value).replace('\n', "↵"), max_cells)
}
