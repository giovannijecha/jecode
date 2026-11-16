//! B14 uses the ordinary B2 card, with remote identity and uncertain outcomes.

use super::{Card, Status, Tool, badge, hint, line_rows};
use crate::{
    json::Value,
    tui::{
        line::Line,
        theme::{BAD, DIM, WARNING},
    },
};

pub(super) fn card(tool: Tool<'_>, columns: usize, keycap: bool) -> Card {
    let status = Status::of(&tool);
    let mut title = badge::badge("MCP call", status);
    title.push(&format!(" {}", target(&tool)), DIM);
    let mut rows = if tool.presentation.expanded {
        super::title::rows("MCP call", &target(&tool), status, columns, true)
    } else if keycap && tool.result.is_some() {
        hint(title, "expand", columns)
    } else {
        vec![title.shortened(columns)]
    };
    let arguments = tool
        .arguments
        .get("arguments")
        .cloned()
        .unwrap_or_else(|| Value::object([]));
    contents(&tool, columns, status, &arguments, &mut rows);
    Card {
        rows,
        hidden: tool.result.is_some() && !tool.presentation.expanded,
    }
}

pub(super) fn target(tool: &Tool<'_>) -> String {
    if let Some(name) = tool
        .result
        .and_then(|r| r.get("connection"))
        .and_then(|r| r.get("name"))
        .and_then(Value::as_str)
    {
        let operation = tool
            .arguments
            .get("tool")
            .or_else(|| tool.arguments.get("action"))
            .and_then(Value::as_str)
            .unwrap_or("list");
        return super::rhythm::inline(&format!("{name} › {operation}"));
    }
    super::rhythm::inline(&crate::events::target("mcp", tool.arguments))
}

fn contents(
    tool: &Tool<'_>,
    columns: usize,
    status: Status,
    arguments: &Value,
    rows: &mut Vec<Line>,
) {
    if tool.presentation.expanded {
        for line in arguments.pretty().lines() {
            rows.extend(line_rows(
                &Line::new(line, DIM),
                columns,
                rows.len(),
                "  │ ",
                "    ",
                DIM,
            ));
        }
        if let Some(result) = tool.result {
            let source = result.get("result").unwrap_or(result).pretty();
            for line in source.lines() {
                rows.extend(line_rows(
                    &Line::new(line, DIM),
                    columns,
                    rows.len(),
                    "  │ ",
                    "    ",
                    DIM,
                ));
            }
            let note = match status {
                Status::Unknown => Some((
                    "Outcome unknown",
                    ". The service may already have acted; check its state before retrying.",
                )),
                Status::Stopped => Some((
                    "Cancelled before dispatch",
                    ". The service did not receive this call.",
                )),
                _ => None,
            };
            if let Some((marker, detail)) = note {
                let mut outcome = Line::new(marker, WARNING);
                outcome.push(detail, DIM);
                rows.extend(line_rows(
                    &outcome,
                    columns,
                    rows.len(),
                    "  └ ",
                    "    ",
                    DIM,
                ));
            } else if status == Status::Failed {
                rows.extend(line_rows(
                    &Line::new("Failed", BAD),
                    columns,
                    rows.len(),
                    "  └ ",
                    "    ",
                    DIM,
                ));
            }
        }
    } else {
        // A call without arguments has no `│` row.
        if let Value::Object(fields) = arguments
            && !fields.is_empty()
        {
            let text = fields
                .iter()
                .map(|(name, value)| format!("{name} {}", value.encode()))
                .collect::<Vec<_>>()
                .join(" · ");
            let line = Line::new(&super::rhythm::inline(&text), DIM)
                .shortened(columns.saturating_sub(4))
                .gutter("  │ ", DIM);
            rows.push(line.shortened(columns));
        }
        if let Some(result) = tool.result {
            let summary = crate::mcp::summary(result);
            let outcome = if status == Status::Failed {
                super::outcome::diagnostic(&summary, status)
            } else {
                super::outcome::line(&summary, status)
            };
            rows.push(
                outcome
                    .shortened(columns.saturating_sub(4))
                    .gutter("  └ ", DIM)
                    .shortened(columns),
            );
        }
    }
}
