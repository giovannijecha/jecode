//! A collaborator's own conversation as transcript items, from its ordered
//! log: each received assignment as a request, its calls and words in the
//! order they happened, and its return receipt last.

use crate::{
    file_change::Change,
    json::Value,
    tui::{
        cards::Presentation,
        state::{Item, Kind},
    },
};
use std::collections::HashMap;

/// The open collaborator's file changes with their hunks, by call id, read
/// from its own session when its record counted `counted` changed calls.
#[derive(Default)]
pub(in crate::tui) struct Changes {
    pub id: String,
    pub counted: usize,
    pub calls: HashMap<String, Change>,
}

/// The record's calls that changed a file.
pub(in crate::tui) fn changed_calls(record: &Value) -> usize {
    super::list(record, "tools")
        .iter()
        .filter(|tool| tool.get("change").is_some())
        .count()
}

fn event(record: &Value, kind: &str, version: usize, details: Vec<String>) -> Value {
    Value::object([
        ("type", Value::string("collaboration")),
        ("kind", Value::string(kind)),
        ("id", record.get("id").cloned().unwrap_or(Value::Null)),
        ("number", Value::number(super::number(record, "number"))),
        ("title", Value::string(super::value(record, "title"))),
        ("version", Value::number(version)),
        (
            "details",
            Value::Array(details.iter().map(Value::string).collect()),
        ),
        ("records", Value::Array(vec![record.clone()])),
    ])
}

fn receipt(event: Value, turn_id: u64) -> Item {
    Item::Tool {
        id: String::new(),
        name: "collaboration".into(),
        arguments: Value::object([
            (
                "label",
                Value::string(format!("Agent {}", super::number(&event, "number"))),
            ),
            ("lone", Value::Bool(true)),
        ]),
        summary: String::new(),
        result: Some(event),
        change: None,
        last: Some(true),
        presentation: Presentation {
            turn_id,
            ..Presentation::default()
        },
    }
}

/// A child call as a finished or running tool row. Calls that never
/// finished in a collaborator that no longer runs have no confirmed result.
/// A change keeps its hunks when the child's session still has them;
/// otherwise its card shows the counts the record kept.
fn call(tool: &Value, working: bool, turn_id: u64, changes: &HashMap<String, Change>) -> Item {
    let result = tool.get("result").cloned().or_else(|| {
        (!working || super::value(tool, "state") != "running")
            .then(|| Value::object([("error", Value::string("No confirmed result"))]))
    });
    let change = tool.get("change").map(|counts| {
        let counted = Change {
            added: super::number(counts, "added"),
            removed: super::number(counts, "removed"),
            created: counts.get("created") == Some(&Value::Bool(true)),
            hunks: Vec::new(),
            ending_change: None,
            final_newline: None,
        };
        let full = changes.get(super::value(tool, "id")).filter(|change| {
            (change.added, change.removed, change.created)
                == (counted.added, counted.removed, counted.created)
        });
        Box::new(full.cloned().unwrap_or(counted))
    });
    Item::Tool {
        id: super::value(tool, "id").into(),
        name: super::value(tool, "name").into(),
        arguments: tool.get("arguments").cloned().unwrap_or(Value::Null),
        summary: super::value(tool, "text").into(),
        result,
        change,
        last: Some(true),
        presentation: Presentation {
            turn_id,
            ..Presentation::default()
        },
    }
}

/// Snapshots without a log show the assignment, calls, messages and outcome.
fn fallback(record: &Value) -> Vec<Value> {
    let entry = |kind: &str, index: usize| {
        Value::object([
            ("kind", Value::string(kind)),
            ("index", Value::number(index)),
        ])
    };
    let mut entries = vec![Value::object([
        ("kind", Value::string("assignment")),
        (
            "version",
            Value::number(super::number(record, "received_version")),
        ),
        (
            "instructions",
            Value::string(super::value(record, "instructions")),
        ),
    ])];
    entries.extend((0..super::list(record, "tools").len()).map(|index| entry("tool", index)));
    entries.extend((0..super::list(record, "updates").len()).map(|index| entry("message", index)));
    let report = record
        .get("report")
        .and_then(|report| report.get("version"));
    match (super::value(record, "state"), report) {
        ("returned" | "failed", Some(version)) => entries.push(Value::object([
            ("kind", Value::string("returned")),
            ("version", version.clone()),
        ])),
        ("stopped", _) => entries.push(Value::object([("kind", Value::string("stopped"))])),
        _ => {}
    }
    entries
}

pub(in crate::tui) fn items(record: &Value, changes: &HashMap<String, Change>) -> Vec<Item> {
    let tools = super::list(record, "tools");
    let updates = super::list(record, "updates");
    let state = super::value(record, "state");
    let working = matches!(state, "running" | "stopping");
    let entries = record
        .get("log")
        .and_then(Value::as_array)
        .map_or_else(|| fallback(record), <[Value]>::to_vec);
    let mut items = Vec::new();
    let mut turn_id = 0;
    for entry in &entries {
        let index = entry.get("index").and_then(Value::as_usize);
        match super::value(entry, "kind") {
            "assignment" => {
                turn_id += 1;
                items.push(Item::Text {
                    kind: Kind::User,
                    text: super::value(entry, "instructions").into(),
                });
            }
            "message" => {
                if let Some(text) = index
                    .and_then(|index| updates.get(index))
                    .and_then(|text| text.as_str())
                {
                    items.push(Item::Text {
                        kind: Kind::Assistant,
                        text: text.into(),
                    });
                }
            }
            "tool" => {
                if let Some(tool) = index.and_then(|index| tools.get(index)) {
                    items.push(call(tool, working, turn_id, changes));
                }
            }
            "returned" => {
                let error = super::value(record, "error");
                let details = (state == "failed" && !error.is_empty())
                    .then(|| error.to_owned())
                    .into_iter()
                    .collect();
                let version = super::number(entry, "version");
                items.push(receipt(
                    event(record, "returned", version, details),
                    turn_id,
                ));
            }
            "stopped" => items.push(receipt(event(record, "stopped", 0, Vec::new()), turn_id)),
            _ => {}
        }
    }
    if state == "unknown" {
        let details = vec!["No confirmed result after recovery".to_string()];
        let version = super::number(record, "received_version");
        items.push(receipt(event(record, "unknown", version, details), turn_id));
    }
    items
}
