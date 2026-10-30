use crate::json::Value;

#[derive(Debug, PartialEq)]
pub enum Event {
    UserQuestion {
        request: crate::ask::Request,
    },
    /// Shown inline by the line-based CLI; the TUI reads the queue itself.
    PermissionRequest {
        request: crate::permissions::Request,
        waiting: usize,
    },
    Collaboration {
        event: Value,
    },
    Preparing,
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
        reason: Option<crate::backend::FailureReason>,
    },
    /// A terminal backend failure's redacted diagnostic and reason.
    BackendFailure {
        reason: crate::backend::FailureReason,
        diagnostic: String,
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
    /// The provider's reported context count, capacity or configured threshold changed.
    Context(crate::context::Status),
    /// A response reported how many tokens it generated, reasoning included.
    Generated {
        tokens: usize,
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
    let detail = text("reason");
    match text("status") {
        Some("partial" | "none") => Some(match detail {
            Some(detail) => format!("checkpoint {}: {detail}", text("status").unwrap_or("")),
            None => format!("checkpoint {}", text("status").unwrap_or("")),
        }),
        _ => detail.map(|detail| format!("checkpoint: {detail}")),
    }
}

fn checkpoint_summary(result: &Value) -> String {
    let count = |key: &str| {
        result
            .get(key)
            .and_then(Value::as_array)
            .map(<[Value]>::len)
    };
    let total = |key: &str, fallback: &str| {
        result
            .get(key)
            .and_then(Value::as_usize)
            .or_else(|| count(fallback))
    };
    let text = |key| result.get(key).and_then(Value::as_str).unwrap_or("");
    let mut summary = if result.get("preview") == Some(&Value::Bool(true)) {
        let planned = total("planned_count", "planned").unwrap_or(0);
        format!("preview: {} would change", counted(planned, "path"))
    } else if let Some(restored) = total("restored_count", "restored") {
        format!(
            "restored {} from {}",
            counted(restored, "path"),
            text("ref")
        )
    } else if let Some(calls) = count("calls") {
        counted(calls, "call")
    } else if let Some(paths) = total("path_count", "paths") {
        format!("inspected {} in {}", counted(paths, "path"), text("ref"))
    } else {
        "complete".into()
    };
    if let Some(conflicts) = total("conflict_count", "conflicts").filter(|count| *count > 0) {
        summary.push_str(&format!("; {}", counted(conflicts, "conflict")));
    }
    if let Some(failed) = total("failed_count", "failed").filter(|count| *count > 0) {
        summary.push_str(&format!("; {} failed", counted(failed, "path")));
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
    if name == "ask_user" {
        return match result.get("outcome").and_then(Value::as_str) {
            Some("answered") => "Answer received",
            Some("dismissed") => "No answer supplied",
            Some("cancelled") => "Question cancelled",
            Some("unavailable") => "Interactive input unavailable",
            _ => "Question finished",
        }
        .into();
    }
    if name == "todo_list" {
        let revision = result
            .get("revision")
            .and_then(Value::as_usize)
            .unwrap_or(0);
        return if revision == 0 {
            "No plan yet".into()
        } else {
            let count = result
                .get("tasks")
                .and_then(Value::as_array)
                .map_or(0, <[Value]>::len);
            format!("{} · revision {revision}", counted(count, "task"))
        };
    }
    if name == "map" {
        match result.get("action").and_then(Value::as_str) {
            Some("remember") => return "Note saved".into(),
            Some("forget") => {
                return if result.get("removed") == Some(&Value::Bool(true)) {
                    "Note forgotten".into()
                } else {
                    "Note not found".into()
                };
            }
            _ => {}
        }
        let count = result
            .get("entries_returned")
            .and_then(Value::as_usize)
            .or_else(|| {
                result
                    .get("entries")
                    .and_then(Value::as_array)
                    .map(<[Value]>::len)
            })
            .unwrap_or(0);
        let mut summary = format!("{count} {}", if count == 1 { "entry" } else { "entries" });
        if let Some(files) = result.get("files").and_then(Value::as_usize) {
            summary.push_str(&format!(" in {}", counted(files, "file")));
        }
        if let Some(notes) = result
            .get("notes_count")
            .and_then(Value::as_usize)
            .filter(|n| *n > 0)
        {
            summary.push_str(&format!("; {}", counted(notes, "note")));
        }
        let errors = result
            .get("errors_count")
            .and_then(Value::as_usize)
            .or_else(|| {
                result
                    .get("errors")
                    .and_then(Value::as_array)
                    .map(<[Value]>::len)
            })
            .unwrap_or(0);
        if errors > 0 {
            summary.push_str(&format!("; {}", counted(errors, "error")));
        }
        if let Some(next) = result.get("next_offset").and_then(Value::as_usize) {
            summary.push_str(&format!("; more from {next}"));
        } else if result.get("complete") == Some(&Value::Bool(false))
            || result.get("inventory_truncated") == Some(&Value::Bool(true))
        {
            summary.push_str("; partial");
        }
        return summary;
    }
    if matches!(name, "list" | "search") {
        let count_key = if name == "list" {
            "entries_returned"
        } else {
            "matches_returned"
        };
        let array_key = if name == "list" { "entries" } else { "matches" };
        let count = result
            .get(count_key)
            .and_then(Value::as_usize)
            .or_else(|| {
                result
                    .get(array_key)
                    .and_then(Value::as_array)
                    .map(<[Value]>::len)
            })
            .unwrap_or(0);
        let mut summary = if name == "list" {
            format!("{count} {}", if count == 1 { "entry" } else { "entries" })
        } else {
            format!("{count} {}", if count == 1 { "match" } else { "matches" })
        };
        if name == "search"
            && let Some(files) = result.get("files_searched").and_then(Value::as_usize)
        {
            summary.push_str(&format!(" in {}", counted(files, "file")));
        }
        let errors = result
            .get("errors_count")
            .and_then(Value::as_usize)
            .or_else(|| {
                result
                    .get("errors")
                    .and_then(Value::as_array)
                    .map(<[Value]>::len)
            })
            .unwrap_or(0);
        if errors > 0 {
            summary.push_str(&format!("; {}", counted(errors, "error")));
        }
        if let Some(next) = result.get("next_offset").and_then(Value::as_usize) {
            summary.push_str(&format!("; more from {next}"));
        } else if result.get("complete") == Some(&Value::Bool(false)) {
            summary.push_str("; partial");
        }
        return summary;
    }
    if name == "checkpoint" {
        return checkpoint_summary(result);
    }
    if name == "browser" {
        if let Some(path) = result.get("screenshot_path").and_then(Value::as_str) {
            return format!("screenshot saved to {path}");
        }
        if let Some(tabs) = result.get("tabs").and_then(Value::as_array) {
            return counted(tabs.len(), "tab");
        }
        if result.get("closed") == Some(&Value::Bool(true)) {
            return "closed".into();
        }
        if result.get("snapshot_path").is_some() {
            return "snapshot captured".into();
        }
        if result.get("logs_path").is_some() {
            return "logs saved".into();
        }
        for (key, summary) in [
            ("clicked", "clicked"),
            ("filled", "filled"),
            ("scrolled", "scrolled"),
            ("found", "found"),
        ] {
            if result.get(key) == Some(&Value::Bool(true)) {
                return summary.into();
            }
        }
        if let Some(key) = result.get("pressed").and_then(Value::as_str) {
            return format!("pressed {key}");
        }
        if result.get("value").is_some() {
            return "script evaluated".into();
        }
        if result.get("result").is_some() {
            return "DevTools call complete".into();
        }
        if let Some(url) = result.get("url").and_then(Value::as_str) {
            return format!("opened {url}");
        }
    }
    if name == "web" {
        if let Some(pages) = result.get("pages").and_then(Value::as_array) {
            let count = result
                .get("pages_in_call")
                .and_then(Value::as_usize)
                .unwrap_or(pages.len());
            let mut summary = format!("crawled {}", counted(count, "page"));
            let failed = pages
                .iter()
                .filter(|page| page.get("error").is_some())
                .count();
            if failed > 0 {
                if result.get("pages_truncated") == Some(&Value::Bool(true)) {
                    summary.push_str(&format!("; {failed} failed in preview"));
                } else {
                    summary.push_str(&format!("; {failed} failed"));
                }
            }
            if result.get("complete") == Some(&Value::Bool(true)) {
                summary.push_str("; complete");
            } else if let Some(queued) = result.get("queued").and_then(Value::as_usize) {
                summary.push_str(&format!("; {queued} queued"));
            }
            if result.get("manifest_repaired") == Some(&Value::Bool(true)) {
                summary.push_str("; manifest repaired");
            }
            return summary;
        }
        if let Some(status) = result.get("status") {
            let bytes = result.get("bytes").and_then(Value::as_usize).unwrap_or(0);
            return format!("HTTP {} · {bytes} bytes", status.encode());
        }
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
                if matches!(name, "bash" | "exec") {
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
    if matches!(name, "bash" | "exec") {
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

mod description;
pub use description::{description, target};

#[cfg(test)]
mod discovery_tests;

#[cfg(test)]
mod tests;
