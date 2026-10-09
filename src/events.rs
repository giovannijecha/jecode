use crate::json::Value;

#[derive(Debug, PartialEq)]
pub enum Event {
    Waiting {
        model: String,
    },
    Message {
        text: String,
    },
    Reasoning,
    Working,
    Recovering {
        attempt: usize,
        delay: std::time::Duration,
        error: String,
    },
    RecoveryFinished,
    RequestDiscarded,
    Maintenance {
        text: String,
    },
    ContextCompacted {
        text: String,
    },
    Streaming {
        text: String,
    },
    ToolStarted {
        id: String,
        name: String,
        arguments: Value,
    },
    ToolFinished {
        id: String,
        name: String,
        summary: String,
        result: Value,
    },
}

pub trait EventSink {
    fn emit(&mut self, event: Event) -> Result<(), String>;
}

impl<F: FnMut(Event) -> Result<(), String>> EventSink for F {
    fn emit(&mut self, event: Event) -> Result<(), String> {
        self(event)
    }
}

pub fn tool_summary(name: &str, result: &Value) -> String {
    if let Some(error) = result.get("error").and_then(Value::as_str) {
        return format!("error: {error}");
    }
    if name == "bash" {
        let mut summary = if result.get("cancelled") == Some(&Value::Bool(true)) {
            "cancelled".into()
        } else if result.get("timed_out") == Some(&Value::Bool(true)) {
            "timed out".into()
        } else {
            format!(
                "exit {}",
                result
                    .get("exit_code")
                    .map_or_else(|| "unknown".into(), Value::encode)
            )
        };
        if result.get("truncated") == Some(&Value::Bool(true)) {
            summary.push_str(if result.get("stdout_file").is_some() {
                "; full output saved"
            } else {
                "; output truncated"
            });
        }
        if let Some(status) = result.get("check_status").and_then(Value::as_str) {
            summary.push_str(&format!("; check {status}"));
        }
        if let Some(changes) = result.get("file_changes").and_then(Value::as_array)
            && !changes.is_empty()
        {
            summary.push_str(&format!("; {} observed file(s) changed", changes.len()));
        }
        if result
            .get("file_tracking")
            .and_then(|tracking| tracking.get("status"))
            .and_then(Value::as_str)
            == Some("incomplete")
        {
            summary.push_str("; file comparison incomplete");
        }
        return summary;
    }
    if name == "protect"
        && let Some(entries) = result.get("file_protections").and_then(Value::as_array)
    {
        let failures = entries
            .iter()
            .filter(|entry| {
                matches!(
                    entry.get("state").and_then(Value::as_str),
                    Some("violated" | "unknown")
                )
            })
            .count();
        return if failures > 0 {
            format!("{} registered files; {failures} unresolved", entries.len())
        } else {
            format!(
                "{} registered file protections; no unresolved changes",
                entries.len()
            )
        };
    }
    if let Some(bytes) = result.get("bytes_written").and_then(Value::as_usize) {
        return format!("wrote {bytes} bytes");
    }
    if result.get("truncated") == Some(&Value::Bool(true)) {
        return "read; more lines available".into();
    }
    "complete".into()
}

pub fn description(name: &str, arguments: &Value) -> String {
    if name == "protect" {
        let action = arguments
            .get("action")
            .and_then(Value::as_str)
            .unwrap_or("invalid action");
        let paths = arguments
            .get("paths")
            .map(Value::encode)
            .unwrap_or_else(|| "all registered files".into());
        return format!("{action} {paths}");
    }
    if name == "bash" {
        return arguments
            .get("command")
            .and_then(Value::as_str)
            .unwrap_or("invalid arguments")
            .into();
    }
    let path = arguments
        .get("path")
        .and_then(Value::as_str)
        .unwrap_or("invalid arguments");
    if name == "read" {
        let offset = arguments
            .get("offset")
            .and_then(Value::as_usize)
            .unwrap_or(1);
        let limit = arguments
            .get("limit")
            .and_then(Value::as_usize)
            .unwrap_or(200);
        return format!("{path} (from line {offset}, up to {limit} lines)");
    }
    path.into()
}
