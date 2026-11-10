//! A collaborator's own conversation in order: received assignments, its
//! visible messages, its calls and its returns. Entries refer to the record's
//! bounded `updates` and its `tools`, so snapshots carry no second copy.

use crate::json::Value;

#[derive(Clone)]
pub(super) enum Entry {
    Assignment {
        version: usize,
        instructions: String,
    },
    /// Ordinal among every visible message; the oldest leave `updates`.
    Message(Option<usize>),
    /// Index into `tools`, which only grows.
    Tool(usize),
    Returned(usize),
    Stopped,
}

/// The ordered log as snapshot entries. `messages` counts every message
/// ordinal handed out; `kept` is how many of the latest remain in `updates`.
pub(super) fn value(entries: &[Entry], messages: usize, kept: usize) -> Value {
    let first = messages.saturating_sub(kept);
    let entry = |pairs: Vec<(&str, Value)>| {
        Value::Object(
            pairs
                .into_iter()
                .map(|(key, value)| (key.into(), value))
                .collect(),
        )
    };
    Value::Array(
        entries
            .iter()
            .map(|item| match item {
                Entry::Assignment {
                    version,
                    instructions,
                } => entry(vec![
                    ("kind", Value::string("assignment")),
                    ("version", Value::number(*version)),
                    ("instructions", Value::string(instructions)),
                ]),
                Entry::Message(ordinal) => {
                    let mut pairs = vec![("kind", Value::string("message"))];
                    if let Some(ordinal) = ordinal.filter(|ordinal| *ordinal >= first) {
                        pairs.push(("index", Value::number(ordinal - first)));
                    }
                    entry(pairs)
                }
                Entry::Tool(index) => entry(vec![
                    ("kind", Value::string("tool")),
                    ("index", Value::number(*index)),
                ]),
                Entry::Returned(version) => entry(vec![
                    ("kind", Value::string("returned")),
                    ("version", Value::number(*version)),
                ]),
                Entry::Stopped => entry(vec![("kind", Value::string("stopped"))]),
            })
            .collect(),
    )
}

/// The log of a saved record and its message ordinal count. Message indices
/// are ordinals into the saved `updates` (`persisted` of them).
pub(super) fn restore(
    record: &Value,
    persisted: usize,
    tools: usize,
) -> Result<(Vec<Entry>, usize), String> {
    let number = |value: &Value, key| value.get(key).and_then(Value::as_usize);
    let saved = record
        .get("log")
        .and_then(Value::as_array)
        .ok_or("Unsupported collaborator log format; file left unchanged")?;
    let entries = saved
        .iter()
        .filter_map(|item| match text(item, "kind").as_str() {
            "assignment" => Some(Entry::Assignment {
                version: number(item, "version").unwrap_or(0),
                instructions: text(item, "instructions"),
            }),
            "message" => Some(Entry::Message(
                number(item, "index").filter(|index| *index < persisted),
            )),
            "tool" => number(item, "index")
                .filter(|index| *index < tools)
                .map(Entry::Tool),
            "returned" => Some(Entry::Returned(number(item, "version").unwrap_or(0))),
            "stopped" => Some(Entry::Stopped),
            _ => None,
        })
        .collect();
    Ok((entries, persisted))
}

fn text(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned()
}
