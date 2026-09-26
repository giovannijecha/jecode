//! Read-only, schema-aware projections of canonical tool receipts.
//! The stored receipt remains untouched; an older shape is still inspectable.
use crate::json::{self, Value};

pub(super) struct Projection {
    pub detail: String,
    pub elapsed_ms: Option<u64>,
}

pub(super) fn project(name: &str, raw: &str) -> Projection {
    let Ok(value) = json::parse(
        raw,
        json::Limits {
            bytes: raw.len().max(1),
            nodes: 100_000,
            depth: 64,
        },
    ) else {
        return Projection {
            detail: raw.into(),
            elapsed_ms: None,
        };
    };
    let elapsed_ms = (name == "run_command")
        .then(|| value.get("elapsed_ms").and_then(Value::unsigned))
        .flatten();
    let detail = match name {
        "run_command" => command(&value),
        "index_receipts" => index(&value),
        "recall_receipts" => recall(&value),
        "list_files" => entries(&value),
        "read_file" => value.get("text").and_then(Value::text).map(str::to_owned),
        "search_text" => matches(&value),
        _ => None,
    }
    .or_else(|| value.get("error").and_then(Value::text).map(str::to_owned))
    .unwrap_or_else(|| raw.into());
    Projection { detail, elapsed_ms }
}

fn json_value(value: &Value) -> String {
    json::encode(value, crate::tools::MAX_OUTPUT.max(1024)).unwrap_or_else(|_| format!("{value:?}"))
}

fn cursor(value: &Value) -> Option<String> {
    let next = value.get("next")?;
    (!matches!(next, Value::Null)).then(|| format!("Next: {}", json_value(next)))
}

fn command(value: &Value) -> Option<String> {
    let status = value.get("status")?.text()?;
    let mut lines = vec![format!("Status: {status}")];
    if let Some(code) = value.get("exit_code").and_then(Value::unsigned) {
        lines.push(format!("Exit code: {code}"));
    } else if let Some(code) = value.get("exit_code")
        && !matches!(code, Value::Null)
    {
        lines.push(format!("Exit code: {}", json_value(code)));
    }
    if let Some(signal) = value.get("signal")
        && !matches!(signal, Value::Null)
    {
        lines.push(format!("Signal: {}", json_value(signal)));
    }
    if value.get("truncated") == Some(&Value::Bool(true)) {
        lines.push("Output truncated".into());
    }
    if let Some(bytes) = value.get("output_bytes").and_then(Value::unsigned) {
        lines.push(format!("Output bytes: {bytes}"));
    }
    if value.get("cleanup_confirmed") == Some(&Value::Bool(false)) {
        lines.push("Cleanup unconfirmed".into());
    }
    for channel in ["stdout", "stderr"] {
        if let Some(text) = value.get(channel).and_then(Value::text)
            && !text.is_empty()
        {
            lines.push(format!("{channel}:"));
            lines.extend(text.lines().map(str::to_owned));
        }
    }
    Some(lines.join("\n"))
}

fn index(value: &Value) -> Option<String> {
    let entries = value.get("entries")?.array()?;
    let mut lines = Vec::new();
    for (position, entry) in entries.iter().enumerate() {
        let name = entry.get("call_name").and_then(Value::text);
        let address = entry.get("recall_address");
        if let (Some(name), Some(address)) = (name, address) {
            lines.push(format!(
                "{}. {name} / recall {}",
                position + 1,
                json_value(address)
            ));
            if entry.get("arguments_omitted") == Some(&Value::Bool(true)) {
                lines.push("   Arguments omitted from index; recall the original result".into());
            } else if let Some(arguments) = entry.get("arguments")
                && !matches!(arguments, Value::Null)
            {
                lines.push(format!("   Arguments: {}", json_value(arguments)));
            }
        } else {
            lines.push(format!("{}. {}", position + 1, json_value(entry)));
        }
    }
    if entries.is_empty() {
        lines.push("No saved reads on this page".into());
    }
    if let Some(next) = cursor(value) {
        lines.push(next);
    }
    Some(lines.join("\n"))
}

fn recall(value: &Value) -> Option<String> {
    let output = value.get("output")?.text()?;
    let mut lines = Vec::new();
    if let (Some(name), Some(id)) = (
        value.get("call_name").and_then(Value::text),
        value.get("call_id").and_then(Value::text),
    ) {
        lines.push(format!("Recorded {name} / call {id}"));
    }
    if let (Some(start), Some(end), Some(total)) = (
        value.get("offset").and_then(Value::unsigned),
        value.get("end_offset").and_then(Value::unsigned),
        value.get("total_bytes").and_then(Value::unsigned),
    ) {
        lines.push(format!("Bytes {start}..{end} of {total}"));
    }
    lines.push(output.into());
    if let Some(next) = cursor(value) {
        lines.push(next);
    }
    Some(lines.join("\n"))
}

fn entries(value: &Value) -> Option<String> {
    let entries = value.get("entries")?.array()?;
    let mut lines: Vec<String> = entries
        .iter()
        .map(|entry| {
            entry.get("name").and_then(Value::text).map_or_else(
                || json_value(entry),
                |name| {
                    format!(
                        "{name}{}",
                        if entry.get("type").and_then(Value::text) == Some("directory") {
                            "/"
                        } else {
                            ""
                        }
                    )
                },
            )
        })
        .collect();
    if let Some(next) = cursor(value) {
        lines.push(next);
    }
    Some(lines.join("\n"))
}

fn matches(value: &Value) -> Option<String> {
    let found = value.get("matches")?.array()?;
    let mut lines: Vec<String> = found
        .iter()
        .map(|entry| {
            match (
                entry.get("path").and_then(Value::text),
                entry.get("line").and_then(Value::unsigned),
                entry.get("text").and_then(Value::text),
            ) {
                (Some(path), Some(line), Some(text)) => format!("{path}:{line}: {text}"),
                _ => json_value(entry),
            }
        })
        .collect();
    if let Some(next) = cursor(value) {
        lines.push(next);
    }
    Some(lines.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_and_index_keep_their_distinct_receipt_shapes() {
        let command = project(
            "run_command",
            r#"{"status":"exit","exit_code":7,"stdout":"first\nseventh marker\n","stderr":"warning\n","truncated":true,"output_bytes":99,"elapsed_ms":1234,"cleanup_confirmed":true}"#,
        );
        assert_eq!(command.elapsed_ms, Some(1234));
        assert!(command.detail.contains("stdout:\nfirst\nseventh marker"));
        assert!(command.detail.contains("stderr:\nwarning"));
        assert!(command.detail.contains("Output truncated"));
        let index = project(
            "index_receipts",
            r#"{"entries":[{"recall_address":{"turn":1,"step":2,"receipt":3,"offset":0,"expected_call_id":"read-1"},"call_name":"read_file","arguments":{"path":"evidence.txt"},"arguments_omitted":false}],"next":{"turn":1,"step":3,"receipt":0}}"#,
        );
        assert!(index.detail.contains("read_file / recall"));
        assert!(index.detail.contains("evidence.txt"));
        assert!(index.detail.contains("Next:"));
        let omitted = project(
            "index_receipts",
            r#"{"entries":[{"recall_address":{"turn":1,"step":2,"receipt":3},"call_name":"read_file","arguments":null,"arguments_omitted":true}],"next":null}"#,
        );
        assert!(omitted.detail.contains("Arguments omitted"));
        let older = project(
            "index_receipts",
            r#"{"entries":[{"legacy_field":"kept"}],"next":null}"#,
        );
        assert!(older.detail.contains("legacy_field"));
    }
}
