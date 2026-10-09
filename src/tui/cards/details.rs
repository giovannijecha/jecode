use super::field;
use crate::{
    json::Value,
    tui::{
        line::Line,
        text,
        theme::{BAD, BODY, DIFF_ADDED_BACKGROUND, DIFF_REMOVED_BACKGROUND, GOOD, MUTED},
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
            "bash" => {
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
                let mut rows = Vec::new();
                for (key, marker, color, background) in [
                    ("old_text", "- ", BAD, DIFF_REMOVED_BACKGROUND),
                    ("new_text", "+ ", GOOD, DIFF_ADDED_BACKGROUND),
                ] {
                    for source in text::clean(field(arguments, key)).lines() {
                        rows.push(Line::new(&format!("{marker}{source}"), color).on(background));
                    }
                }
                rows
            }
            _ => payload(&result.pretty(), BODY),
        }
    };
    rows.into_iter()
        .enumerate()
        .flat_map(|(index, row)| row.at_source(index + 1).wrap(columns, false))
        .collect()
}

fn payload(value: &str, style: &'static str) -> Vec<Line> {
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
