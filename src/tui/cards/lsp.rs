//! B15 language-server calls use the existing B2 card grammar.
use super::{Card, Status, Tool, badge, hint, line_rows};
use crate::{
    events::diagnostic_counts,
    json::Value,
    tui::{
        line::Line,
        text,
        theme::{BAD, BODY, CODE_TEXT, DIM, MUTED, WARNING},
    },
};

pub(super) fn card(tool: Tool<'_>, columns: usize, keycap: bool) -> Card {
    let status = Status::of(&tool);
    let mut title = badge::badge("LSP", status);
    let label = if tool.presentation.expanded {
        "collapse"
    } else {
        "expand"
    };
    let cap = if keycap && tool.result.is_some() {
        8 + 1 + label.len() + 2
    } else {
        0
    };
    let target_width = columns.saturating_sub(text::cells(&title.plain()) + 1 + cap);
    title.push(&format!(" {}", target(&tool, target_width)), BODY);
    let mut rows = if keycap && tool.result.is_some() {
        hint(title, label, columns)
    } else {
        vec![title.shortened(columns)]
    };
    if tool.presentation.expanded {
        for line in expanded_arguments(&tool).pretty().lines() {
            rows.extend(line_rows(
                &Line::new(line, CODE_TEXT),
                columns,
                rows.len(),
                "  │ ",
                "    ",
                DIM,
            ));
        }
        if let Some(result) = tool.result {
            for line in result.pretty().lines() {
                rows.extend(line_rows(
                    &Line::new(line, tone(status)),
                    columns,
                    rows.len(),
                    "  │ ",
                    "    ",
                    DIM,
                ));
            }
            rows.extend(line_rows(
                &summary_line(result, tool.arguments, status),
                columns,
                rows.len(),
                "  └ ",
                "    ",
                DIM,
            ));
        }
    } else {
        let arguments = compact_arguments(tool.arguments);
        if !arguments.is_empty() {
            rows.push(
                Line::new(&arguments, MUTED)
                    .shortened(columns.saturating_sub(4))
                    .gutter("  │ ", DIM)
                    .shortened(columns),
            );
        }
        if let Some(result) = tool.result {
            rows.push(
                summary_line(result, tool.arguments, status)
                    .shortened(columns.saturating_sub(4))
                    .gutter("  └ ", DIM)
                    .shortened(columns),
            );
        }
    }
    Card {
        rows,
        hidden: tool.result.is_some() && !tool.presentation.expanded,
    }
}

fn expanded_arguments(tool: &Tool<'_>) -> Value {
    let mut arguments = tool.arguments.clone();
    if arguments.get("server").is_none()
        && let Some(server) = tool
            .result
            .and_then(|result| result.get("server"))
            .and_then(Value::as_str)
        && let Value::Object(fields) = &mut arguments
    {
        fields.insert("resolved_server_id".into(), Value::string(server));
    }
    arguments
}

fn tone(status: Status) -> crate::tui::theme::Style {
    match status {
        Status::Failed => BAD,
        Status::Stopped | Status::Unknown => WARNING,
        _ => MUTED,
    }
}

/// The action and Ctrl+O survive narrow rows; the server name yields space first.
pub(super) fn target(tool: &Tool<'_>, width: usize) -> String {
    let action = tool
        .arguments
        .get("action")
        .and_then(Value::as_str)
        .unwrap_or("status");
    let server = tool
        .result
        .and_then(|result| result.get("server_name"))
        .and_then(Value::as_str)
        .or_else(|| tool.arguments.get("server_name").and_then(Value::as_str))
        .or_else(|| {
            tool.result
                .and_then(|result| result.get("server"))
                .and_then(Value::as_str)
        })
        .or_else(|| tool.arguments.get("server").and_then(Value::as_str))
        .unwrap_or("server");
    let action_width = text::cells(action) + 3;
    let room = width.saturating_sub(action_width);
    if room < 3 {
        return action.into();
    }
    format!("{} › {action}", text::ellipsize(&text::clean(server), room))
}

fn compact_arguments(arguments: &Value) -> String {
    let path = arguments.get("path").and_then(Value::as_str).unwrap_or("");
    let line = arguments.get("line").map(Value::encode).unwrap_or_default();
    let column = arguments
        .get("column")
        .map(Value::encode)
        .unwrap_or_default();
    let mut parts = Vec::new();
    if !path.is_empty() {
        parts.push(if line.is_empty() {
            path.into()
        } else {
            format!("{path}:{line}:{column}")
        });
    }
    if arguments.get("include_declaration") == Some(&Value::Bool(true)) {
        parts.push("include declaration".into());
    }
    text::clean(&parts.join(" · "))
}

fn summary_line(result: &Value, arguments: &Value, status: Status) -> Line {
    let action = arguments
        .get("action")
        .and_then(Value::as_str)
        .unwrap_or("");
    if action != "diagnostics"
        || status != Status::Done
        || result.get("status").and_then(Value::as_str) != Some("ready")
    {
        return Line::new(&summary_for(result, action), tone(status));
    }
    let freshness = result
        .get("freshness")
        .and_then(Value::as_str)
        .unwrap_or("unavailable");
    if freshness != "current" {
        return Line::new(
            &summary_for(result, action),
            if matches!(freshness, "stale" | "sync_gap") {
                WARNING
            } else {
                MUTED
            },
        );
    }
    let Some((errors, warnings, _)) = diagnostic_counts(result) else {
        return Line::new(&summary_for(result, action), MUTED);
    };
    let mut line = Line::new(
        &format!("{errors} errors"),
        if errors > 0 { BAD } else { MUTED },
    );
    line.push(" · ", MUTED);
    line.push(
        &format!("{warnings} warnings"),
        if warnings > 0 { WARNING } else { MUTED },
    );
    line.push(
        &format!(
            " · {}",
            result
                .get("freshness")
                .and_then(Value::as_str)
                .unwrap_or("unverified")
        ),
        MUTED,
    );
    if result.get("complete") == Some(&Value::Bool(false)) {
        line.push(" · partial, incomplete", WARNING);
    }
    if result.get("truncated") == Some(&Value::Bool(true)) {
        line.push(" · more in result file", MUTED);
    }
    line
}

pub(super) fn summary_for(result: &Value, action: &str) -> String {
    // A failed call has no partial result, only its error.
    if let Some(error) = result
        .get("error")
        .and_then(Value::as_str)
        .filter(|error| !error.is_empty())
    {
        return text::clean(error);
    }
    let status = result
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or("ready");
    let detail = result.get("detail").and_then(Value::as_str).unwrap_or("");
    let freshness = result
        .get("freshness")
        .and_then(Value::as_str)
        .unwrap_or("");
    let complete = result.get("complete") != Some(&Value::Bool(false));
    // Totals recorded before inline truncation outlive the arrays.
    let locations = result
        .get("total_locations")
        .and_then(Value::as_usize)
        .or_else(|| {
            result
                .get("locations")
                .and_then(Value::as_array)
                .map(<[_]>::len)
        });
    let diagnostics = diagnostic_counts(result);
    let text = match status {
        "off" => "Server off".into(),
        "not_started" => "Not started".into(),
        "starting" => "Starting".into(),
        "not_found" => "Executable not found".into(),
        "unsupported" => "Capability not advertised".into(),
        "timed_out" => "Timed out".into(),
        "cancelled" => "Cancelled".into(),
        "settings_changed" => "Stopped · settings changed".into(),
        "crashed" => "Server exited".into(),
        "restart_suppressed" => "Automatic restart paused · Allow restart in Settings".into(),
        "configuration_error" => "Configuration error".into(),
        "invalid_input" => "Invalid request".into(),
        "protocol_error" => "Protocol error".into(),
        "unavailable" => "Server unavailable".into(),
        "failed" => "Failed".into(),
        _ if freshness == "sync_gap" => "Sync incomplete".into(),
        _ if action == "diagnostics"
            || action.is_empty() && diagnostics.is_some_and(|(_, _, total)| total > 0) =>
        {
            match (freshness, diagnostics) {
                ("current", Some((errors, warnings, _))) => {
                    format!("{errors} errors · {warnings} warnings · current")
                }
                ("current", None) => "Diagnostics current · counts unavailable".into(),
                ("pending", _) => "Diagnostics pending".into(),
                ("stale", Some((_, _, total))) => format!("Stale diagnostics · {total} findings"),
                ("stale", None) => "Stale diagnostics".into(),
                ("unverified", Some((_, _, total))) => {
                    format!("Unverified version · {total} findings")
                }
                ("unverified", None) => "Unverified version".into(),
                _ => "Diagnostics unavailable".into(),
            }
        }
        _ if let Some(count @ 1..) = locations => format!(
            "{count} {}",
            if count == 1 { "location" } else { "locations" }
        ),
        _ if matches!(action, "definition" | "references") && locations.is_none() => {
            "Locations unavailable".into()
        }
        _ if matches!(action, "definition" | "references") => "No locations".into(),
        _ if action == "status" => format!(
            "{} configured",
            result
                .get("servers")
                .and_then(Value::as_array)
                .map_or(0, <[_]>::len)
        ),
        _ => "Available".into(),
    };
    let mut text = text;
    if !complete {
        text.push_str(" · partial, incomplete");
    }
    if result.get("truncated") == Some(&Value::Bool(true)) {
        text.push_str(" · more in result file");
    }
    if !detail.is_empty() {
        text.push_str(" · ");
        text.push_str(detail);
    }
    text::clean(&text)
}

#[cfg(test)]
mod tests;
