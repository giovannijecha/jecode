use crate::json::Value;

#[derive(Debug, PartialEq)]
pub enum Event {
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

mod description;
pub use description::{description, target};

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

    #[test]
    fn checkpoint_summaries_use_actual_preview_and_full_counts() {
        let restored = Value::object([
            ("ref", Value::string("rc-one")),
            ("preview", Value::Bool(false)),
            ("planned", Value::Array(vec![Value::object([])])),
            ("planned_count", Value::number(71)),
            ("restored", Value::Array(vec![Value::object([])])),
            ("restored_count", Value::number(60)),
            ("conflicts", Value::Array(vec![])),
            ("conflict_count", Value::number(0)),
            ("failed", Value::Array(vec![])),
            ("failed_count", Value::number(0)),
        ]);
        assert_eq!(
            tool_summary("checkpoint", &restored),
            "restored 60 paths from rc-one"
        );
        let mut preview = restored;
        if let Value::Object(fields) = &mut preview {
            fields.insert("preview".into(), Value::Bool(true));
        }
        assert_eq!(
            tool_summary("checkpoint", &preview),
            "preview: 71 paths would change"
        );
    }

    #[test]
    fn checkpoint_summaries_distinguish_list_inspect_and_failures() {
        let listed = Value::object([("calls", Value::Array(vec![Value::object([])]))]);
        assert_eq!(tool_summary("checkpoint", &listed), "1 call");
        let inspected = Value::object([
            ("ref", Value::string("rc-one")),
            ("paths", Value::Array(vec![Value::object([])])),
            ("path_count", Value::number(80)),
        ]);
        assert_eq!(
            tool_summary("checkpoint", &inspected),
            "inspected 80 paths in rc-one"
        );
        let failed = Value::object([
            ("ref", Value::string("rc-one")),
            ("preview", Value::Bool(false)),
            ("restored_count", Value::number(1)),
            ("failed", Value::Array(vec![Value::object([])])),
            ("failed_count", Value::number(3)),
            ("conflict_count", Value::number(2)),
        ]);
        assert_eq!(
            tool_summary("checkpoint", &failed),
            "restored 1 path from rc-one; 2 conflicts; 3 paths failed"
        );
    }

    #[test]
    fn local_web_and_browser_cards_show_actions_and_artifacts() {
        let fetch = Value::object([
            ("action", Value::string("fetch")),
            ("url", Value::string("https://example.org/")),
        ]);
        assert_eq!(description("web", &fetch), "fetch https://example.org/");
        let fetched = Value::object([("status", Value::number(200)), ("bytes", Value::number(42))]);
        assert_eq!(tool_summary("web", &fetched), "HTTP 200 · 42 bytes");

        let screenshot = Value::object([
            ("action", Value::string("screenshot")),
            ("tab_id", Value::string("tab-1")),
        ]);
        assert_eq!(description("browser", &screenshot), "screenshot tab-1");
        let captured = Value::object([("screenshot_path", Value::string("tmp:browser/shot.png"))]);
        assert_eq!(
            tool_summary("browser", &captured),
            "screenshot saved to tmp:browser/shot.png"
        );
        let crawl = Value::object([
            ("pages_in_call", Value::number(35)),
            ("pages_truncated", Value::Bool(true)),
            ("manifest_repaired", Value::Bool(true)),
            ("queued", Value::number(4)),
            (
                "pages",
                Value::Array(vec![Value::object([("error", Value::string("offline"))])]),
            ),
        ]);
        assert_eq!(
            tool_summary("web", &crawl),
            "crawled 35 pages; 1 failed in preview; 4 queued; manifest repaired"
        );
    }

    #[test]
    fn routine_description_overrides_command_and_blank_falls_back() {
        for (name, key, command, fallback) in [
            (
                "read",
                "path",
                "src/lib.rs",
                "src/lib.rs (from line 1, up to 200 lines)",
            ),
            ("bash", "command", "cargo test", "cargo test"),
            ("process", "action", "list", "list"),
            ("web", "action", "fetch", "fetch"),
            ("browser", "action", "tabs", "tabs"),
        ] {
            let arguments = Value::object([
                (key, Value::string(command)),
                ("description", Value::string("Inspect relevant results")),
            ]);
            assert_eq!(
                description(name, &arguments),
                "Inspect relevant results",
                "{name}"
            );
            assert_eq!(target(name, &arguments), fallback, "{name}");
            for blank in ["", " \t "] {
                let arguments = Value::object([
                    (key, Value::string(command)),
                    ("description", Value::string(blank)),
                ]);
                assert_eq!(description(name, &arguments), fallback, "{name}");
            }
            let arguments = Value::object([(key, Value::string(command))]);
            assert_eq!(description(name, &arguments), fallback, "{name}");
        }
    }
}
