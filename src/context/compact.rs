//! What a tool result tells the model. Records keep every field for the
//! interface, exports and history reads; requests carry only the fields that
//! say something, because each one is sent again with every later request.
//! The view depends only on the record, so request prefixes stay identical.

use crate::json::{self, Value};
use std::collections::BTreeMap;

pub(super) fn compact(message: &Value) -> Value {
    let Some(content) = message.get("content").and_then(Value::as_str) else {
        return message.clone();
    };
    let Ok(mut result) = json::parse(content) else {
        return message.clone();
    };
    if !matches!(result, Value::Object(_)) {
        return message.clone();
    }
    trim(&mut result);
    let mut message = message.clone();
    if let Value::Object(fields) = &mut message {
        fields.insert("content".into(), Value::string(result.encode()));
    }
    message
}

fn trim(result: &mut Value) {
    let Value::Object(fields) = result else {
        return;
    };
    // Saved output files and sizes matter only when the text here is partial
    // or the process is still running.
    let partial = fields.contains_key("process_id")
        || ["truncated", "stdout_truncated", "stderr_truncated"]
            .iter()
            .any(|key| fields.get(*key) == Some(&Value::Bool(true)));
    fields.retain(|key, value| !noise(key, value, partial));
    if let Some(Value::Object(checkpoint)) = fields.get_mut("checkpoint")
        && checkpoint.get("status").and_then(Value::as_str) == Some("saved")
    {
        checkpoint.retain(|key, _| key == "ref");
    }
    if let Some(Value::Object(skipped)) = fields.get_mut("skipped") {
        skipped.retain(|_, count| !matches!(count, Value::Number(number) if number == "0"));
        if skipped.is_empty() {
            fields.remove("skipped");
        }
    }
    if let Some(text) = fields
        .get("matches")
        .and_then(Value::as_array)
        .filter(|matches| !matches.is_empty())
        .and_then(grep_lines)
    {
        fields.insert("matches".into(), Value::string(text));
    }
    for key in ["files", "steps", "entries", "tasks"] {
        if let Some(Value::Array(items)) = fields.get_mut(key) {
            items.iter_mut().for_each(trim);
        }
    }
    if let Some(inner) = fields.get_mut("result") {
        trim(inner);
    }
}

/// Fields whose value only restates the default, or that no later call can use.
fn noise(key: &str, value: &Value, partial: bool) -> bool {
    match value {
        Value::Null => true,
        Value::Bool(false) => matches!(
            key,
            "cancelled"
                | "timed_out"
                | "truncated"
                | "stdout_truncated"
                | "stderr_truncated"
                | "partial_line"
                | "errors_truncated"
        ),
        Value::String(text) if text.is_empty() => matches!(key, "stdout" | "stderr" | "note"),
        Value::String(_) => matches!(key, "stdout_file" | "stderr_file") && !partial,
        Value::Array(items) => items.is_empty() && key == "errors",
        Value::Number(number) => match key {
            "output_limit_bytes" | "bytes_returned" | "lines_returned" => true,
            "stdout_bytes" | "stderr_bytes" => !partial,
            "errors_count" | "notes_count" | "byte_offset" => number == "0",
            "offset" => number == "1",
            _ => false,
        },
        Value::Object(fields) => {
            key == "checkpoint" && fields.get("status").and_then(Value::as_str) == Some("none")
        }
        _ => false,
    }
}

/// One file's lines by number: whether each is a match, and its text.
type Lines = BTreeMap<usize, (bool, String)>;

/// Search matches in the form every model knows from grep: the path once, then
/// `line:text` for matches and `line-text` for context, with `--` between gaps.
fn grep_lines(matches: &[Value]) -> Option<String> {
    let mut files: Vec<(&str, Lines)> = Vec::new();
    for entry in matches {
        let path = entry.get("path")?.as_str()?;
        let position = match files.iter().position(|(known, _)| *known == path) {
            Some(position) => position,
            None => {
                files.push((path, BTreeMap::new()));
                files.len() - 1
            }
        };
        let lines = &mut files[position].1;
        for key in ["before", "after"] {
            for context in entry.get(key).and_then(Value::as_array).unwrap_or(&[]) {
                lines
                    .entry(context.get("line")?.as_usize()?)
                    .or_insert((false, line_text(context)?));
            }
        }
        lines.insert(entry.get("line")?.as_usize()?, (true, line_text(entry)?));
    }
    let mut text = String::new();
    for (path, lines) in files {
        text.push_str(path);
        text.push('\n');
        let mut previous = None;
        for (line, (matched, content)) in lines {
            if previous.is_some_and(|previous: usize| line > previous + 1) {
                text.push_str("--\n");
            }
            let mark = if matched { ':' } else { '-' };
            text.push_str(&format!("{line}{mark}{content}\n"));
            previous = Some(line);
        }
    }
    Some(text)
}

fn line_text(line: &Value) -> Option<String> {
    let text = line.get("text")?.as_str()?;
    if line.get("text_truncated") != Some(&Value::Bool(true)) {
        return Some(text.into());
    }
    Some(match line.get("byte_offset").and_then(Value::as_usize) {
        Some(offset) => format!("{text} [line cut; read from byte_offset {offset}]"),
        None => format!("{text} [line cut]"),
    })
}

#[cfg(test)]
mod tests;
