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
    /// Messages submitted during the turn joined the conversation.
    Accepted {
        prompts: Vec<crate::attachments::Prompt>,
    },
    /// The turn waits for an edit that holds back a submitted message.
    InputHeld,
    /// The projected next request changed: input, usage, tools, reduction or capacity.
    Context(crate::context::Status),
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
    ToolChanged {
        id: String,
        change: crate::file_change::Change,
    },
    /// Context for a running call, such as the checkpoint before a command.
    ToolNote {
        id: String,
        text: String,
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
    let summary = outcome_summary(name, result);
    match checkpoint_warning(result) {
        Some(warning) => format!("{summary}; {warning}"),
        None => summary,
    }
}

/// A checkpoint problem that people should see next to a tool result.
pub fn checkpoint_warning(result: &Value) -> Option<String> {
    let checkpoint = result.get("checkpoint")?;
    let text = |key| checkpoint.get(key).and_then(Value::as_str);
    if text("status") == Some("unprotected") {
        return Some(format!("unprotected: {}", text("reason").unwrap_or("")));
    }
    let detail = text("warning").or(text("error")).or(text("reason"));
    match text("status") {
        Some("partial" | "none") => Some(match detail {
            Some(detail) => format!("checkpoint {}: {detail}", text("status").unwrap_or("")),
            None => format!("checkpoint {}", text("status").unwrap_or("")),
        }),
        _ => detail.map(|detail| format!("checkpoint: {detail}")),
    }
}

fn checkpoint_summary(result: &Value) -> String {
    if let Some(saved) = result.get("saved").and_then(Value::as_usize) {
        let skipped = result.get("skipped").and_then(Value::as_usize).unwrap_or(0);
        let unprotected = result
            .get("unprotected")
            .and_then(Value::as_usize)
            .unwrap_or(0);
        let mut summary = format!(
            "captured {}; {skipped} skipped; {unprotected} unprotected",
            counted(saved, "path")
        );
        if result.get("point_error").is_some() {
            summary.push_str("; point unavailable");
        }
        return summary;
    }
    let count = |key| {
        result
            .get(key)
            .and_then(Value::as_array)
            .map(<[Value]>::len)
    };
    let text = |key| result.get(key).and_then(Value::as_str).unwrap_or("");
    let mut summary = if let Some(planned) = count("planned") {
        format!("preview: {} would change", counted(planned, "path"))
    } else if let Some(restored) = count("restored") {
        format!(
            "restored {} from {}",
            counted(restored, "path"),
            text("target")
        )
    } else if let Some(points) = count("points") {
        counted(points, "point")
    } else if let Some(paths) = count("paths") {
        format!("history of {}", counted(paths, "path"))
    } else if let Some(state) = result.get("state").and_then(Value::as_str) {
        state.into()
    } else if result.get("pinned").is_some() {
        text("ref").into()
    } else {
        "complete".into()
    };
    if let Some(conflicts) = count("conflicts") {
        summary.push_str(&format!("; {}", counted(conflicts, "conflict")));
    }
    if result.get("failed").is_some() {
        summary.push_str("; stopped by an error");
    }
    summary
}

fn counted(count: usize, noun: &str) -> String {
    format!("{count} {noun}{}", if count == 1 { "" } else { "s" })
}

fn outcome_summary(name: &str, result: &Value) -> String {
    if let Some(error) = result.get("error").and_then(Value::as_str) {
        return format!("error: {error}");
    }
    if name == "checkpoint" {
        return checkpoint_summary(result);
    }
    if name == "process"
        && let Some(processes) = result.get("processes").and_then(Value::as_array)
    {
        let running = processes
            .iter()
            .filter(|process| process_status(process) == Some("running"))
            .count();
        return format!("{} processes, {running} running", processes.len());
    }
    if let Some(status) = process_status(result) {
        let id = result
            .get("process_id")
            .and_then(Value::as_str)
            .unwrap_or("");
        return match status {
            "running" => format!(
                "{} · {id}",
                if name == "bash" {
                    "started in background"
                } else {
                    "running at last check"
                }
            ),
            "stopping" => format!("stopping in background · {id}"),
            "exited" => format!(
                "exit {}",
                result
                    .get("exit_code")
                    .map_or_else(|| "unknown".into(), Value::encode)
            ),
            "timed_out" => "timed out".into(),
            status => status.into(),
        };
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
        return summary;
    }
    if let Some(bytes) = result.get("bytes_written").and_then(Value::as_usize) {
        return format!("wrote {bytes} bytes");
    }
    if result.get("truncated") == Some(&Value::Bool(true)) {
        return "read; more lines available".into();
    }
    "complete".into()
}

/// The status of a persistent process result, if it is one.
pub fn process_status(result: &Value) -> Option<&str> {
    result
        .get("process_id")
        .and(result.get("status"))
        .and_then(Value::as_str)
}

pub fn description(name: &str, arguments: &Value) -> String {
    if name == "process" {
        let field = |key| arguments.get(key).and_then(Value::as_str).unwrap_or("");
        return format!("{} {}", field("action"), field("process_id"))
            .trim_end()
            .into();
    }
    if name == "checkpoint" {
        let field = |key| arguments.get(key).and_then(Value::as_str).unwrap_or("");
        let mut words = vec![field("action"), field("ref"), field("label")];
        if let Some(paths) = arguments.get("paths").and_then(Value::as_array) {
            words.extend(paths.iter().filter_map(Value::as_str));
        }
        words.retain(|word| !word.is_empty());
        return words.join(" ");
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bash_summary_exposes_incomplete_checkpoint_status() {
        for status in ["partial", "none"] {
            let result = Value::object([
                ("exit_code", Value::number(0)),
                (
                    "checkpoint",
                    Value::object([("status", Value::string(status))]),
                ),
            ]);
            let summary = tool_summary("bash", &result);
            assert!(
                summary.contains(status),
                "{status} checkpoint was hidden from the tool card: {summary}"
            );
        }
    }
}
