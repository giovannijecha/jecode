use super::{diff, field};
use crate::{
    json::Value,
    tui::{
        line::Line,
        text,
        theme::{BAD, BODY, MUTED, Style},
    },
};

pub(super) fn render(
    name: &str,
    arguments: &Value,
    result: &Value,
    columns: usize,
    failed: bool,
) -> Vec<Line> {
    let rows = if let Some(error) = result.get("error").and_then(Value::as_str) {
        payload(error, BAD)
    } else {
        match name {
            "bash" | "process" if result.get("processes").is_none() => {
                let mut rows = payload(field(result, "stdout"), BODY);
                let stderr = field(result, "stderr");
                if !stderr.is_empty() {
                    rows.push(Line::new("stderr", if failed { BAD } else { MUTED }));
                    rows.extend(payload(stderr, if failed { BAD } else { MUTED }));
                }
                rows
            }
            "read" => payload(field(result, "content"), BODY),
            "write" => payload(field(arguments, "content"), BODY),
            "edit" => {
                let old = text::clean(field(arguments, "old_text"));
                let new = text::clean(field(arguments, "new_text"));
                diff::lines(&old, &new)
                    .into_iter()
                    .map(|(change, line)| diff::row(change, line))
                    .collect()
            }
            _ => payload(&result.pretty(), BODY),
        }
    };
    rows.into_iter()
        .enumerate()
        .flat_map(|(index, row)| row.at_source(index + 1).wrap(columns, false))
        .collect()
}

fn payload(value: &str, style: Style) -> Vec<Line> {
    if value.is_empty() {
        return Vec::new();
    }
    let value = text::clean(value);
    value
        .strip_suffix('\n')
        .unwrap_or(&value)
        .split('\n')
        .map(|line| Line::new(line, style))
        .collect()
}
