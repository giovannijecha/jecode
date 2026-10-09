use super::{field, flag};
use crate::json::Value;
use crate::tui::{
    line::Line,
    text,
    theme::{BAD, BODY, DIFF_ADDED_BACKGROUND, DIFF_REMOVED_BACKGROUND, GOOD, MUTED, WARNING},
};

const ROWS: usize = 3;

pub(super) struct Preview {
    pub rows: Vec<Line>,
    pub total: usize,
}

pub(super) fn render(
    name: &str,
    arguments: &Value,
    result: &Value,
    columns: usize,
    failed: bool,
) -> Preview {
    if let Some(error) = result.get("error").and_then(Value::as_str) {
        return head(payload(error, BAD, columns));
    }
    match name {
        "bash" => bash(result, columns, failed),
        "read" => head(payload(field(result, "content"), BODY, columns)),
        "write" => head(payload(field(arguments, "content"), BODY, columns)),
        "edit" => edit(arguments, columns),
        _ => head(payload(&result.pretty(), BODY, columns)),
    }
}

fn head(mut rows: Vec<Line>) -> Preview {
    let total = rows.len();
    rows.truncate(ROWS);
    Preview { rows, total }
}

fn bash(result: &Value, columns: usize, failed: bool) -> Preview {
    let stdout = payload(field(result, "stdout"), BODY, columns);
    let stderr = if field(result, "stderr").is_empty() {
        Vec::new()
    } else {
        payload(
            &format!("stderr: {}", field(result, "stderr")),
            if failed { BAD } else { MUTED },
            columns,
        )
    };
    let mut rows = Vec::new();
    if failed && !stderr.is_empty() {
        rows.extend(stderr.iter().take(ROWS).cloned());
        rows.extend(tail(&stdout, ROWS - rows.len()));
    } else {
        rows.extend(tail(
            &stdout,
            if stderr.is_empty() { ROWS } else { ROWS - 1 },
        ));
        rows.extend(stderr.iter().take(ROWS - rows.len()).cloned());
    }
    Preview {
        rows,
        total: stdout.len() + stderr.len(),
    }
}

fn tail(rows: &[Line], count: usize) -> impl Iterator<Item = Line> + '_ {
    rows.iter().skip(rows.len().saturating_sub(count)).cloned()
}

fn edit(arguments: &Value, columns: usize) -> Preview {
    let removed = diff(field(arguments, "old_text"), false, columns);
    let added = diff(field(arguments, "new_text"), true, columns);
    let removed_budget = if added.is_empty() {
        ROWS
    } else if removed.len() > added.len() {
        2
    } else {
        1
    };
    let removed_count = removed.len().min(removed_budget);
    let added_count = added.len().min(ROWS - removed_count);
    let removed_count = removed.len().min(ROWS - added_count);
    let rows = removed
        .iter()
        .take(removed_count)
        .chain(added.iter().take(added_count))
        .cloned()
        .collect::<Vec<_>>();
    Preview {
        rows,
        total: removed.len() + added.len(),
    }
}

pub(super) fn notices(name: &str, result: &Value, columns: usize) -> Vec<Line> {
    let mut rows = Vec::new();
    if name == "read"
        && result.get("offset") == Some(&Value::Null)
        && let Some(next) = result.get("next_byte_offset").and_then(Value::as_usize)
    {
        rows.extend(payload(
            &format!("More file bytes available from byte {next}."),
            MUTED,
            columns,
        ));
        return rows;
    }
    if name == "read"
        && let Some(next) = result.get("next_offset").and_then(Value::as_usize)
    {
        if flag(result, "partial_line") {
            rows.extend(payload(
                &format!(
                    "Line continues at byte {}.",
                    result
                        .get("next_byte_offset")
                        .and_then(Value::as_usize)
                        .unwrap_or(0)
                ),
                MUTED,
                columns,
            ));
            return rows;
        }
        rows.extend(payload(
            &format!("More file lines available from line {next}."),
            MUTED,
            columns,
        ));
    }
    if name == "bash" {
        for stream in ["stdout", "stderr"] {
            if flag(result, &format!("{stream}_truncated")) {
                let bytes = result
                    .get(&format!("{stream}_bytes"))
                    .and_then(Value::as_usize)
                    .unwrap_or(0);
                let limit = result
                    .get("output_limit_bytes")
                    .and_then(Value::as_usize)
                    .unwrap_or(0);
                let saved = field(result, &format!("{stream}_file"));
                let notice = if saved.is_empty() {
                    format!("{stream} capture truncated: {limit} of {bytes} bytes retained.")
                } else {
                    format!("{stream}: last {limit} of {bytes} bytes shown · full output saved.")
                };
                rows.extend(payload(&notice, WARNING, columns));
            }
        }
    }
    rows
}

fn payload(value: &str, style: &'static str, columns: usize) -> Vec<Line> {
    if value.is_empty() {
        return Vec::new();
    }
    let value = text::clean(value);
    // One final newline terminates the last output line; additional ones are
    // actual empty output rows and count toward the compact preview.
    value
        .strip_suffix('\n')
        .unwrap_or(&value)
        .split('\n')
        .map(|line| Line::new(line, style).shortened(columns))
        .collect()
}

fn diff(value: &str, added: bool, columns: usize) -> Vec<Line> {
    let value = text::clean(value);
    value
        .lines()
        .map(|value| {
            Line::new(
                &format!("{} {value}", if added { "+" } else { "-" }),
                if added { GOOD } else { BAD },
            )
            .on(if added {
                DIFF_ADDED_BACKGROUND
            } else {
                DIFF_REMOVED_BACKGROUND
            })
            .shortened(columns)
        })
        .collect()
}
