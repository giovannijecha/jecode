//! Shared work has no isolated baseline. Its recorded edit hunks are still
//! useful evidence, without attributing other writers' changes to the agent.

use crate::{
    collaboration::Observer,
    file_change::{Change, Kind},
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
    let Some(path) = path else {
        fields.insert(
            "files".into(),
            record
                .get("files")
                .cloned()
                .unwrap_or(Value::Array(Vec::new())),
        );
        return Ok(result);
    };
    let tools = record.get("tools").and_then(Value::as_array).unwrap_or(&[]);
    let mut text = String::new();
    for (call, change) in observer.changes(id) {
        if change.files.is_empty() {
            let changed_path = tools
                .iter()
                .find(|tool| tool.get("id").and_then(Value::as_str) == Some(&call))
                .and_then(|tool| tool.get("arguments"))
                .and_then(|arguments| arguments.get("path"))
                .and_then(Value::as_str);
            if changed_path == Some(path) {
                append(&mut text, &call, path, &change);
            }
        } else if let Some(change) = change.file(path) {
            append(&mut text, &call, path, change);
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

fn append(text: &mut String, call: &str, path: &str, change: &Change) {
    let _ = writeln!(
        text,
        "{call}: {path} (+{} -{})",
        change.added, change.removed
    );
    for hunk in &change.hunks {
        text.push_str("@@\n");
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
    if change.hunks.is_empty() {
        text.push_str("Hunks not recorded; read the current file.\n");
    }
}
