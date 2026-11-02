//! B14 uses the ordinary B2 card, with remote identity and uncertain outcomes.

use super::{Card, Status, Tool, badge, hint, line_rows};
use crate::{
    json::Value,
    tui::{
        line::Line,
        text,
        theme::{BAD, BODY, CODE_TEXT, DIM, MUTED, WARNING},
    },
};

pub(super) fn card(tool: Tool<'_>, columns: usize, keycap: bool) -> Card {
    let status = Status::of(&tool);
    let mut title = badge::badge("MCP", status);
    title.push(&format!(" {}", target(&tool)), BODY);
    let mut rows = if keycap && tool.result.is_some() {
        hint(
            title,
            if tool.presentation.expanded {
                "collapse"
            } else {
                "expand"
            },
            columns,
        )
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
                &Line::new(line, CODE_TEXT),
                columns,
                rows.len(),
                "  │ ",
                "    ",
                DIM,
            ));
        }
        if let Some(result) = tool.result {
            let style = match status {
                Status::Unknown | Status::Stopped => WARNING,
                Status::Failed => BAD,
                _ => MUTED,
            };
            let source = result.get("result").unwrap_or(result).pretty();
            for line in source.lines() {
                rows.extend(line_rows(
                    &Line::new(line, style),
                    columns,
                    rows.len(),
                    "  │ ",
                    "    ",
                    DIM,
                ));
            }
            let text = if status == Status::Unknown {
                "Outcome unknown. The service may already have acted; check its state before retrying."
            } else if status == Status::Stopped {
                "Cancelled before dispatch. The service did not receive this call."
            } else {
                ""
            };
            if !text.is_empty() {
                rows.extend(line_rows(
                    &Line::new(text, WARNING),
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
            let line = Line::new(&super::rhythm::inline(&text), MUTED)
                .shortened(columns.saturating_sub(4))
                .gutter("  │ ", DIM);
            rows.push(line.shortened(columns));
        }
        if let Some(result) = tool.result {
            let style = match status {
                Status::Unknown | Status::Stopped => WARNING,
                Status::Failed => BAD,
                _ => MUTED,
            };
            rows.push(
                Line::new(&text::clean(&crate::mcp::summary(result)), style)
                    .shortened(columns.saturating_sub(4))
                    .gutter("  └ ", DIM)
                    .shortened(columns),
            );
        }
    }
}
