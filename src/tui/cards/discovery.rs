//! Structured file discovery receipts. The tool's JSON stays out of the TUI.

use super::{Body, line_rows};
use crate::{
    json::Value,
    tui::{
        line::Line,
        text,
        theme::{BAD, CODE_TEXT, DIM, MUTED, WARNING},
    },
};

pub(super) fn render(name: &str, result: &Value, columns: usize, expanded: bool) -> Body {
    if !expanded {
        return Body::new(Vec::new());
    }
    let mut rows = Vec::new();
    if name == "list" {
        for entry in array(result, "entries") {
            let kind = match field(entry, "type") {
                "directory" => "dir",
                "symlink" => "link",
                "file" => "file",
                _ => "other",
            };
            let depth = number(entry, "depth").min(6);
            let path = single_line(field(entry, "path"), 240);
            let size = entry
                .get("size")
                .and_then(Value::as_usize)
                .map_or_else(String::new, |size| format!("  {size} B"));
            let line = format!("{}{}  {path}{size}", "  ".repeat(depth), kind);
            push(&mut rows, &line, columns, CODE_TEXT);
        }
    } else {
        for matched in array(result, "matches") {
            let path = single_line(field(matched, "path"), 200);
            let line = number(matched, "line");
            let source = single_line(field(matched, "text"), 400);
            context(&mut rows, matched, "before", "  <", columns);
            let clipped = if matched.get("text_truncated") == Some(&Value::Bool(true)) {
                " …"
            } else {
                ""
            };
            push(
                &mut rows,
                &format!("{path}:{line}: {source}{clipped}"),
                columns,
                CODE_TEXT,
            );
            context(&mut rows, matched, "after", "  >", columns);
        }
    }
    if rows.is_empty() {
        push(
            &mut rows,
            if name == "list" {
                "No entries"
            } else {
                "No matches"
            },
            columns,
            DIM,
        );
    }
    let skipped = result.get("skipped");
    let skipped: Vec<_> = ["hidden", "ignored", "links", "binary", "non_utf8", "other"]
        .into_iter()
        .filter_map(|key| {
            let count = skipped
                .and_then(|value| value.get(key))
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
        let path = single_line(field(error, "path"), 200);
        let detail = single_line(field(error, "error"), 300);
        push(&mut rows, &format!("Error {path}: {detail}"), columns, BAD);
    }
    let shown_errors = array(result, "errors").len();
    let total_errors = result
        .get("errors_count")
        .and_then(Value::as_usize)
        .unwrap_or(shown_errors);
    if total_errors > shown_errors {
        push(
            &mut rows,
            &format!("{} more errors not shown", total_errors - shown_errors),
            columns,
            BAD,
        );
    }
    if result.get("complete") == Some(&Value::Bool(false)) {
        let continuation = result
            .get("next_offset")
            .and_then(Value::as_usize)
            .map_or_else(
                || "Partial result".into(),
                |next| format!("More from {next}"),
            );
        push(&mut rows, &continuation, columns, WARNING);
    }
    Body::new(rows)
}

fn context(rows: &mut Vec<Line>, matched: &Value, key: &str, prefix: &str, columns: usize) {
    for entry in array(matched, key) {
        let line = number(entry, "line");
        let source = single_line(field(entry, "text"), 300);
        let clipped = if entry.get("text_truncated") == Some(&Value::Bool(true)) {
            " …"
        } else {
            ""
        };
        push(
            rows,
            &format!("{prefix}{line}: {source}{clipped}"),
            columns,
            MUTED,
        );
    }
}

fn push(rows: &mut Vec<Line>, value: &str, columns: usize, style: crate::tui::theme::Style) {
    let source = rows.len() + 2;
    rows.extend(line_rows(
        &Line::new(value, style),
        columns,
        source,
        "    ",
        "    ",
        DIM,
    ));
}

fn array<'a>(value: &'a Value, key: &str) -> &'a [Value] {
    value.get(key).and_then(Value::as_array).unwrap_or(&[])
}

fn field<'a>(value: &'a Value, key: &str) -> &'a str {
    value.get(key).and_then(Value::as_str).unwrap_or("")
}

fn number(value: &Value, key: &str) -> usize {
    value.get(key).and_then(Value::as_usize).unwrap_or(0)
}

fn single_line(value: &str, max_cells: usize) -> String {
    let clean = text::clean(value).replace('\n', "↵");
    text::ellipsize(&clean, max_cells)
}
