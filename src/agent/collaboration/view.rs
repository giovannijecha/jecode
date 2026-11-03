//! The collaborator view the main model receives from the agent tool. Session
//! events keep complete records for restore and display; the model gets state,
//! the report and short evidence lines, so a return never floods its context.
//! A collaborator's full work stays in its own conversation.

use crate::json::Value;

const FILES: usize = 40;
const CHECKS: usize = 12;
const TOOLS: usize = 12;
const UPDATES: usize = 3;
const LINE: usize = 160;
const TEXT: usize = 1500;

/// Maps an agent tool result to the model view. `inspect` adds recent work.
pub(super) fn result(action: &str, id: Option<&str>, value: Value) -> Value {
    if action == "inspect" {
        return record(&value, true);
    }
    let Value::Object(mut fields) = value else {
        return value;
    };
    for internal in ["after_message", "sequence", "in_call", "in_turn"] {
        fields.remove(internal);
    }
    if let Some(Value::Array(records)) = fields.get("records") {
        // A receipt for one collaborator (review, stop) carries every record;
        // only the addressed one belongs to this call.
        let compact = records
            .iter()
            .filter(|item| id.is_none_or(|id| item.get("id").and_then(Value::as_str) == Some(id)))
            .map(|item| record(item, false))
            .collect();
        fields.insert("records".into(), Value::Array(compact));
    }
    Value::Object(fields)
}

/// State, report and evidence of one collaborator, with bounded lists.
pub(super) fn record(record: &Value, detail: bool) -> Value {
    let mut view = Vec::new();
    for key in [
        "id",
        "number",
        "title",
        "state",
        "workspace",
        "workspace_closed",
        "sent_version",
        "received_version",
        "activity",
        "reviewed",
        "error",
        "report",
    ] {
        if let Some(value) = record.get(key).filter(|value| **value != Value::Null) {
            view.push((key, value.clone()));
        }
    }
    if record
        .get("pending_instruction")
        .is_some_and(|value| *value != Value::Null)
    {
        view.push(("assignment_pending", Value::Bool(true)));
    }
    let list = |key| record.get(key).and_then(Value::as_array).unwrap_or(&[]);
    let files = list("files");
    view.push((
        "files",
        Value::Array(files.iter().take(FILES).cloned().collect()),
    ));
    if files.len() > FILES {
        view.push(("files_total", Value::number(files.len())));
    }
    let checks = list("checks");
    view.push((
        "checks",
        Value::Array(checks[checks.len().saturating_sub(CHECKS)..].to_vec()),
    ));
    if checks.len() > CHECKS {
        view.push(("checks_total", Value::number(checks.len())));
    }
    let tools = list("tools");
    view.push(("tool_calls", Value::number(tools.len())));
    let updates = list("updates");
    let has_report = record
        .get("report")
        .is_some_and(|report| *report != Value::Null);
    let shown_updates = if detail {
        UPDATES
    } else {
        usize::from(!has_report)
    };
    if shown_updates > 0 && !updates.is_empty() {
        view.push((
            "updates",
            Value::Array(
                updates[updates.len().saturating_sub(shown_updates)..]
                    .iter()
                    .filter_map(Value::as_str)
                    .map(|text| Value::string(clip(text, TEXT)))
                    .collect(),
            ),
        ));
    }
    if detail {
        if let Some(text) = record.get("instructions").and_then(Value::as_str) {
            view.push(("instructions", Value::string(clip(text, TEXT))));
        }
        view.push((
            "recent_tools",
            Value::Array(
                tools[tools.len().saturating_sub(TOOLS)..]
                    .iter()
                    .map(|tool| Value::string(tool_line(tool)))
                    .collect(),
            ),
        ));
    }
    Value::Object(
        view.into_iter()
            .map(|(key, value)| (key.to_owned(), value))
            .collect(),
    )
}

/// One line per tool call: name, main argument and outcome.
fn tool_line(tool: &Value) -> String {
    let name = tool.get("name").and_then(Value::as_str).unwrap_or("tool");
    let arguments = tool.get("arguments");
    let target = ["path", "command", "action", "url", "query", "pattern"]
        .iter()
        .find_map(|key| arguments?.get(key)?.as_str())
        .map(str::to_owned)
        .or_else(|| {
            let executable = arguments?.get("executable")?.as_str()?;
            let args = arguments?
                .get("args")
                .and_then(Value::as_array)
                .map(|args| {
                    args.iter()
                        .filter_map(Value::as_str)
                        .collect::<Vec<_>>()
                        .join(" ")
                })
                .unwrap_or_default();
            Some(format!("{executable} {args}"))
        })
        .unwrap_or_default();
    let result = tool.get("result");
    let outcome = if let Some(error) = result.and_then(|result| result.get("error")?.as_str()) {
        format!("error: {error}")
    } else if let Some(Value::Number(code)) = result.and_then(|result| result.get("exit_code")) {
        format!("exit {code}")
    } else if let Some(status) = result.and_then(|result| result.get("status")?.as_str()) {
        status.to_owned()
    } else if let Some(text) = result.and_then(Value::as_str) {
        text.to_owned()
    } else {
        "ok".to_owned()
    };
    clip(
        &format!("{name} {} → {}", clip(&target, LINE / 2), outcome)
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" "),
        LINE,
    )
}

fn clip(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        return text.to_owned();
    }
    let mut clipped = text.chars().take(limit).collect::<String>();
    clipped.push('…');
    clipped
}

#[cfg(test)]
mod tests;
