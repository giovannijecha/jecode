//! Shared work has no isolated baseline. Its recorded edit hunks are still
//! useful evidence, without attributing other writers' changes to the agent.

use crate::{
    collaboration::Observer,
    file_change::{Change, Entry, Kind},
    json::Value,
};
use std::fmt::Write;

pub(super) fn shared(
    observer: &Observer,
    id: &str,
    path: Option<&str>,
    offset: usize,
    limit: usize,
) -> Result<Value, String> {
    let record = observer
        .snapshot()
        .into_iter()
        .find(|record| record.get("id").and_then(Value::as_str) == Some(id))
        .ok_or("Unknown collaborator")?;
    let mut result = Value::object([
        ("workspace", Value::string("shared")),
        ("source", Value::string("recorded_tool_changes")),
        ("complete", Value::Bool(false)),
        (
            "note",
            Value::string(
                "Recorded edit history, not a project-wide baseline or the current file. Command changes and later writes may be absent; read current files to verify them. No integration is needed for shared work.",
            ),
        ),
    ]);
    let Value::Object(fields) = &mut result else {
        unreachable!()
    };
    let files = record
        .get("files")
        .cloned()
        .unwrap_or(Value::Array(Vec::new()));
    let Some(path) = path else {
        fields.insert("files".into(), files);
        return Ok(result);
    };
    if !files
        .as_array()
        .is_some_and(|files| files.contains(&Value::string(path)))
    {
        return Err("Path has no recorded edit from this collaborator".into());
    }
    let tools = record.get("tools").and_then(Value::as_array).unwrap_or(&[]);
    let mut text = String::new();
    for (call, change) in observer.changes(id) {
        if !change.files.is_empty() {
            if let Some(change) = change.file(path) {
                append(&mut text, &call, path, change);
            }
            continue;
        }
        let named = named_paths(tools, &call);
        if !named.contains(&path) {
            continue;
        }
        if named.iter().all(|named| *named == path) {
            append(&mut text, &call, path, &change);
        } else {
            let _ = writeln!(
                text,
                "{call}: {path}: edit omitted because this call ID was reused for other files; read the current file."
            );
        }
    }
    let start = text.ceil_char_boundary(offset.min(text.len()));
    let end = text.floor_char_boundary(start.saturating_add(limit).min(text.len()));
    // Even a one-byte budget must make progress over a multi-byte character.
    let end = if end == start && start < text.len() {
        text.ceil_char_boundary(start + 1)
    } else {
        end
    };
    fields.extend([
        ("path".into(), Value::string(path)),
        ("offset".into(), Value::number(start)),
        ("diff".into(), Value::string(&text[start..end])),
        ("total_bytes".into(), Value::number(text.len())),
        (
            "next_offset".into(),
            if end < text.len() {
                Value::number(end)
            } else {
                Value::Null
            },
        ),
    ]);
    Ok(result)
}

/// The paths named by the changed calls with this ID. Providers may reuse an
/// ID, which then cannot say which of several paths a saved change belongs to.
fn named_paths<'a>(tools: &'a [Value], call: &str) -> Vec<&'a str> {
    tools
        .iter()
        .filter(|tool| {
            tool.get("change").is_some() && tool.get("id").and_then(Value::as_str) == Some(call)
        })
        .filter_map(|tool| tool.get("arguments")?.get("path")?.as_str())
        .collect()
}

/// A unified diff range: the hunk's first line and how many lines it holds.
fn range(hunk: &[Entry], line: impl Fn(&Entry) -> Option<usize>) -> String {
    let mut lines = hunk.iter().filter_map(line);
    let first = lines.next();
    format!(
        "{},{}",
        first.unwrap_or(0),
        usize::from(first.is_some()) + lines.count()
    )
}

fn append(text: &mut String, call: &str, path: &str, change: &Change) {
    let _ = writeln!(
        text,
        "{call}: {path} (+{} -{})",
        change.added, change.removed
    );
    for hunk in &change.hunks {
        let _ = writeln!(
            text,
            "@@ -{} +{} @@",
            range(hunk, |entry| entry.old_line),
            range(hunk, |entry| entry.new_line)
        );
        for entry in hunk {
            text.push(match entry.kind {
                Kind::Context => ' ',
                Kind::Removed => '-',
                Kind::Added => '+',
            });
            text.push_str(&entry.text);
            text.push('\n');
            if entry.no_newline {
                text.push_str("\\ No newline at end of file\n");
            }
        }
    }
    if let Some(ending) = &change.ending_change {
        let _ = writeln!(text, "Line endings: {ending}");
    }
    if let Some(present) = change.final_newline {
        let _ = writeln!(
            text,
            "Final newline: {}",
            if present { "added" } else { "removed" }
        );
    }
}
