//! Checkpoint results as labeled rows for people: the `/checkpoints` panel,
//! plain output, `jecode checkpoints` and the checkpoint tool card.
use crate::json::Value;

/// Fields shown first in a row, in this order; others follow by name.
const ORDER: [&str; 12] = [
    "ref",
    "path",
    "tool",
    "kind",
    "action",
    "saved",
    "current",
    "expected",
    "created_at",
    "expires_at",
    "bytes",
    "reason",
];
/// Fields that identify a row and label it.
const IDENTIFIERS: [&str; 2] = ["ref", "path"];

/// A titled list of labeled rows.
pub struct Output {
    pub title: String,
    pub details: Vec<(String, String)>,
    /// Exact content printed instead of rows, such as a recorded file.
    pub raw: Option<String>,
}

impl Output {
    /// Aligned rows, or the exact content, for plain text.
    pub fn text(&self) -> String {
        if let Some(raw) = &self.raw {
            return raw.clone();
        }
        let width = self
            .details
            .iter()
            .map(|(label, _)| label.chars().count())
            .max()
            .unwrap_or(0)
            .min(28);
        let mut lines = vec![self.title.clone()];
        lines.extend(self.details.iter().map(|(label, value)| {
            if value.is_empty() {
                label.clone()
            } else {
                format!("{label:<width$}  {value}")
            }
        }));
        lines.join("\n")
    }
}

pub fn output(title: String, value: &Value, prefix: &str) -> Output {
    Output {
        title,
        details: details(value, prefix),
        raw: None,
    }
}

pub fn text(value: &Value, prefix: &str) -> String {
    output(String::new(), value, prefix)
        .text()
        .trim_start_matches('\n')
        .into()
}

pub fn details(value: &Value, prefix: &str) -> Vec<(String, String)> {
    let Value::Object(fields) = value else {
        return vec![(String::new(), scalar(value))];
    };
    let mut rows = Vec::new();
    for (key, value) in fields {
        let name = label(key);
        match (key.as_str(), value) {
            ("undo", Value::Object(_)) => rows.push(("Undo".into(), command(value, prefix))),
            ("retry", Value::Object(retry)) => {
                let note = retry.get("note").and_then(Value::as_str).unwrap_or("");
                let expected = retry.get("expected").and_then(Value::as_str).unwrap_or("");
                rows.push(("Retry".into(), format!("{note} (--expected {expected})")));
            }
            (_, Value::Array(items)) if items.is_empty() => rows.push((name, "none".into())),
            (_, Value::Array(items)) if items.iter().all(|item| !is_object(item)) => {
                rows.push((
                    name,
                    items.iter().map(scalar).collect::<Vec<_>>().join(", "),
                ));
            }
            (_, Value::Array(items)) => {
                rows.push((name, items.len().to_string()));
                for item in items {
                    entry(item, "  ", &mut rows);
                }
            }
            (_, Value::Object(_)) => rows.push((name, row(value))),
            _ => rows.push((name, scalar(value))),
        }
    }
    rows
}

/// A field name as a label: `created_at` reads "Created at".
fn label(key: &str) -> String {
    let name = key.replace('_', " ");
    let mut characters = name.chars();
    characters.next().map_or(name.clone(), |first| {
        first.to_uppercase().chain(characters).collect()
    })
}

fn is_object(value: &Value) -> bool {
    matches!(value, Value::Object(_))
}

/// One object as a row labeled by its identifier; nested lists follow.
fn entry(value: &Value, indent: &str, rows: &mut Vec<(String, String)>) {
    let Value::Object(fields) = value else {
        rows.push((indent.into(), scalar(value)));
        return;
    };
    let label = IDENTIFIERS
        .iter()
        .find_map(|key| fields.get(*key).and_then(Value::as_str))
        .unwrap_or("");
    rows.push((format!("{indent}{label}"), row(value)));
    for item in fields
        .values()
        .filter_map(Value::as_array)
        .flatten()
        .filter(|item| is_object(item))
    {
        entry(item, &format!("{indent}  "), rows);
    }
}

fn scalar(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Number(number) => number.clone(),
        Value::Bool(flag) => if *flag { "yes" } else { "no" }.into(),
        Value::Null => "none".into(),
        other => other.encode(),
    }
}

/// An object's fields on one line, the most identifying first.
fn row(value: &Value) -> String {
    let Value::Object(fields) = value else {
        return scalar(value);
    };
    let mut entries: Vec<(&String, &Value)> = fields
        .iter()
        .filter(|(key, value)| {
            !IDENTIFIERS.contains(&key.as_str())
                && !matches!(value, Value::Array(items) if items.iter().any(is_object))
        })
        .collect();
    entries.sort_by_key(|(key, _)| {
        ORDER
            .iter()
            .position(|known| known == key)
            .unwrap_or(ORDER.len())
    });
    entries
        .into_iter()
        .map(|(key, value)| match (key.as_str(), value) {
            ("label" | "action" | "state", _) => scalar(value),
            (_, Value::Array(items)) => format!(
                "{}: {}",
                key.replace('_', " "),
                items.iter().map(scalar).collect::<Vec<_>>().join(", ")
            ),
            _ => format!("{}: {}", key.replace('_', " "), scalar(value)),
        })
        .filter(|text| !text.is_empty())
        .collect::<Vec<_>>()
        .join(" · ")
}

/// The command that repeats an action such as an undo.
fn command(value: &Value, prefix: &str) -> String {
    let action = value.get("action").and_then(Value::as_str).unwrap_or("");
    let mut words = vec![prefix.to_string(), action.replace('_', "-")];
    words.extend(value.get("ref").and_then(Value::as_str).map(String::from));
    if let Some(expected) = value.get("expected").and_then(Value::as_str) {
        words.push("--expected".into());
        words.push(quote(expected, prefix));
    }
    if let Some(paths) = value.get("paths").and_then(Value::as_array)
        && !paths.is_empty()
    {
        words.push("--".into());
        words.extend(
            paths
                .iter()
                .filter_map(Value::as_str)
                .map(|path| quote(path, prefix)),
        );
    }
    words.join(" ")
}

/// Quote for the TUI parser or for a pasted CLI command in a native shell.
pub(super) fn quote(path: &str, prefix: &str) -> String {
    if prefix != "/checkpoints" {
        #[cfg(windows)]
        return format!("'{}'", path.replace('\'', "''"));
        #[cfg(not(windows))]
        return format!("'{}'", path.replace('\'', "'\\''"));
    }
    if path.contains(char::is_whitespace) || path.contains('"') {
        format!("\"{}\"", path.replace('\\', "\\\\").replace('"', "\\\""))
    } else {
        path.into()
    }
}
