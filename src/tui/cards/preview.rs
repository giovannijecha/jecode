use super::{
    diff::{self, Change},
    field, flag,
};
use crate::json::Value;
use crate::tui::{
    line::Line,
    notice::{self, Level},
    text,
    theme::{BAD, BODY, MUTED, Style},
};

const ROWS: usize = 3;

pub(super) fn render(
    name: &str,
    arguments: &Value,
    result: &Value,
    columns: usize,
    failed: bool,
) -> Preview {
    if let Some(error) = result.get("error").and_then(Value::as_str) {
        return head(&payload(error, BAD, columns));
    }
    match name {
        "bash" | "process" if result.get("processes").is_none() => bash(result, columns, failed),
        "read" => head(&payload(field(result, "content"), BODY, columns)),
        "write" => head(&payload(field(arguments, "content"), BODY, columns)),
        "edit" => edit(arguments, columns),
        _ => head(&payload(&result.pretty(), BODY, columns)),
    }
}

pub(super) struct Preview {
    pub rows: Vec<Line>,
    pub position: Option<String>,
}

fn selected(rows: Vec<Line>, total: usize) -> Preview {
    let position = (total > rows.len()).then(|| format!("preview {} / {total}", rows.len()));
    Preview { rows, position }
}

fn head(rows: &[Line]) -> Preview {
    selected(rows.iter().take(ROWS).cloned().collect(), rows.len())
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
    // Failed stderr takes priority; recent stdout fills any remaining space.
    let rows = if failed && !stderr.is_empty() {
        let mut rows = stderr.iter().take(ROWS).cloned().collect::<Vec<_>>();
        let room = ROWS - rows.len();
        rows.extend(
            stdout
                .iter()
                .skip(stdout.len().saturating_sub(room))
                .cloned(),
        );
        rows
    } else {
        let budget = if stderr.is_empty() { ROWS } else { ROWS - 1 };
        let mut rows = stdout
            .iter()
            .skip(stdout.len().saturating_sub(budget))
            .cloned()
            .collect::<Vec<_>>();
        rows.extend(stderr.iter().take(ROWS - rows.len()).cloned());
        rows
    };
    selected(rows, stdout.len() + stderr.len())
}

fn edit(arguments: &Value, columns: usize) -> Preview {
    let old = text::clean(field(arguments, "old_text"));
    let new = text::clean(field(arguments, "new_text"));
    let changes = diff::lines(&old, &new);
    let removed = changes
        .iter()
        .take_while(|(change, _)| *change == Change::Removed)
        .count();
    let added = changes[removed..]
        .iter()
        .take_while(|(change, _)| *change == Change::Added)
        .count();
    let budget = if added == 0 {
        ROWS
    } else if removed > added {
        2
    } else {
        1
    };
    let first = removed.min(budget);
    let second = added.min(ROWS - first);
    let first = removed.min(ROWS - second);
    let rows = changes[..first]
        .iter()
        .chain(&changes[removed..removed + second])
        .map(|(change, line)| diff::row(*change, line).shortened(columns))
        .collect();
    selected(rows, changes.len())
}

pub(super) fn notices(name: &str, result: &Value, columns: usize) -> Vec<Line> {
    let mut rows = Vec::new();
    if name == "read"
        && result.get("offset") == Some(&Value::Null)
        && let Some(next) = result.get("next_byte_offset").and_then(Value::as_usize)
    {
        rows.extend(notice::lines(
            Level::Info,
            &format!("More file bytes available from byte {next}."),
            columns,
        ));
        return rows;
    }
    if name == "read"
        && let Some(next) = result.get("next_offset").and_then(Value::as_usize)
    {
        if flag(result, "partial_line") {
            rows.extend(notice::lines(
                Level::Info,
                &format!(
                    "Line continues at byte {}.",
                    result
                        .get("next_byte_offset")
                        .and_then(Value::as_usize)
                        .unwrap_or(0)
                ),
                columns,
            ));
            return rows;
        }
        rows.extend(notice::lines(
            Level::Info,
            &format!("More file lines available from line {next}."),
            columns,
        ));
    }
    if matches!(name, "bash" | "process") {
        for stream in ["stdout", "stderr"] {
            let skipped = result
                .get(&format!("{stream}_skipped_bytes"))
                .and_then(Value::as_usize)
                .unwrap_or(0);
            if skipped > 0 {
                let notice =
                    format!("{stream}: {skipped} earlier bytes not shown · full output saved.");
                rows.extend(notice::lines(Level::Warning, &notice, columns));
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
                let saved = field(result, &format!("{stream}_file"));
                let notice = if saved.is_empty() {
                    format!("{stream} capture truncated: {limit} of {bytes} bytes retained.")
                } else {
                    format!("{stream}: last {limit} of {bytes} bytes shown · full output saved.")
                };
                rows.extend(notice::lines(Level::Warning, &notice, columns));
            }
        }
    }
    rows
}

fn payload(value: &str, style: Style, columns: usize) -> Vec<Line> {
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
