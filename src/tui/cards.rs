use super::{
    activity,
    line::Line,
    text,
    theme::{
        BAD, BODY, EMPHASIS, GOOD, MUTED, TOOL_ACCENT, TOOL_GUIDE, TOOL_SELECTED, TOOL_SURFACE,
        WARNING,
    },
};
use crate::json::Value;

mod details;
mod presentation;
mod preview;
pub use presentation::Presentation;

pub struct Tool<'a> {
    pub name: &'a str,
    pub arguments: &'a Value,
    pub summary: &'a str,
    pub result: Option<&'a Value>,
    pub last: bool,
    pub preview: bool,
    pub presentation: &'a Presentation,
}

pub fn render(tool: Tool<'_>, columns: usize) -> Vec<Line> {
    let stopped = tool.result.is_some_and(|value| flag(value, "cancelled"));
    let failed = tool.result.is_some_and(|value| {
        value.get("error").is_some()
            || flag(value, "timed_out")
            || tool.name == "bash"
                && value
                    .get("exit_code")
                    .is_some_and(|code| code != &Value::number(0))
    });
    let style = if tool.result.is_none() {
        TOOL_ACCENT
    } else if stopped {
        WARNING
    } else if failed {
        BAD
    } else {
        GOOD
    };
    let outcome = outcome(&tool, failed || stopped);
    let (output, preview) = output(&tool, columns.saturating_sub(5), failed || stopped);
    let mut status = Line::default();
    if tool.result.is_some() {
        status.push(
            if stopped {
                "■ "
            } else if failed {
                "× "
            } else {
                "✓ "
            },
            style,
        );
    }
    status.push(
        &text::ellipsize(&text::clean(&outcome), (columns / 2).max(12)),
        style,
    );
    if let Some(elapsed) = tool.presentation.duration() {
        status.push(&format!("  ·  {}", activity::duration(elapsed)), style);
    }
    if !preview.is_empty()
        && text::cells(&status.plain()) + text::cells(&preview) + 3 <= columns.saturating_sub(5)
    {
        status.push(&format!(" · {preview}"), MUTED);
    }
    let branch = if tool.last { "└─ " } else { "├─ " };
    let guide = if tool.result.is_none() || tool.presentation.selected {
        TOOL_ACCENT
    } else {
        TOOL_GUIDE
    };
    let continuation = if tool.last { "     " } else { "│    " };
    let description = field(
        tool.arguments,
        if tool.name == "bash" {
            "command"
        } else {
            "path"
        },
    );
    let description = text::clean(description).replace('\n', " ↵ ");
    let target_style = if tool.result.is_none() || tool.presentation.selected {
        TOOL_ACCENT
    } else {
        BODY
    };
    let mut command = Line::new(&format!("{}  ", text::clean(tool.name)), EMPHASIS).at_source(0);
    let mut rows = if columns < 72 {
        command.push(&description, target_style);
        let mut rows: Vec<_> = command
            .wrap(columns.saturating_sub(3), false)
            .into_iter()
            .enumerate()
            .map(|(index, line)| {
                let prefix = if index == 0 {
                    branch
                } else if tool.last {
                    "   "
                } else {
                    "│  "
                };
                surface(line.gutter(prefix, guide), &tool).shortened(columns)
            })
            .collect();
        rows.push(surface(status.gutter(continuation, TOOL_GUIDE), &tool).shortened(columns));
        rows
    } else {
        let mut header = command.gutter(branch, guide);
        let status_cells = text::cells(&status.plain());
        let budget = columns.saturating_sub(text::cells(&header.plain()) + status_cells + 3);
        header.push(&text::ellipsize(&description, budget), target_style);
        header.push(
            &" ".repeat(columns.saturating_sub(text::cells(&header.plain()) + status_cells + 1)),
            MUTED,
        );
        header.spans.extend(status.spans);
        vec![surface(header, &tool).shortened(columns)]
    };
    if tool.result.is_some() {
        let mut source_line = 1;
        rows.extend(output.into_iter().map(|mut row| {
            if let Some(origin) = row.origin {
                source_line = source_line.max(origin.line + 1);
            } else {
                row = row.at_source(source_line);
                source_line += 1;
            }
            row.gutter(continuation, TOOL_GUIDE).shortened(columns)
        }));
    }
    rows
}

fn output(tool: &Tool<'_>, columns: usize, failed: bool) -> (Vec<Line>, String) {
    let Some(result) = tool.result else {
        return (Vec::new(), String::new());
    };
    let show = tool.presentation.expanded.unwrap_or(tool.preview || failed);
    let (mut rows, position) = if !show {
        (Vec::new(), String::new())
    } else if tool.presentation.expanded == Some(true) {
        (
            details::render(tool.name, tool.arguments, result, columns, failed),
            String::new(),
        )
    } else {
        let preview = preview::render(tool.name, tool.arguments, result, columns, failed);
        let position = if preview.rows.len() < preview.total {
            format!("preview {} / {}", preview.rows.len(), preview.total)
        } else {
            String::new()
        };
        (preview.rows, position)
    };
    rows.extend(preview::notices(tool.name, result, columns));
    (rows, position)
}

fn surface(mut line: Line, tool: &Tool<'_>) -> Line {
    if tool.presentation.selected {
        line.background = Some(TOOL_SELECTED);
    } else if tool.result.is_none() {
        line.background = Some(TOOL_SURFACE);
    }
    line
}

fn outcome(tool: &Tool<'_>, failed: bool) -> String {
    if tool.result.is_none() {
        return "running".into();
    }
    let result = tool.result.unwrap();
    if result.get("error").is_some() {
        return "failed".into();
    }
    if tool.name != "read" || failed {
        return tool.summary.into();
    }
    if tool.arguments.get("byte_offset").is_some() {
        let start = result
            .get("byte_offset")
            .and_then(Value::as_usize)
            .unwrap_or(0);
        let count = result
            .get("bytes_returned")
            .and_then(Value::as_usize)
            .unwrap_or(0);
        return if count == 0 {
            "no bytes returned".into()
        } else {
            format!("bytes {start}–{}", start.saturating_add(count - 1))
        };
    }
    result
        .get("lines_returned")
        .and_then(Value::as_usize)
        .map_or_else(
            || tool.summary.into(),
            |count| {
                let start = result
                    .get("offset")
                    .or_else(|| tool.arguments.get("offset"))
                    .and_then(Value::as_usize)
                    .unwrap_or(1);
                if count == 0 {
                    "no lines returned".into()
                } else {
                    format!("lines {start}–{}", start.saturating_add(count - 1))
                }
            },
        )
}

fn flag(value: &Value, key: &str) -> bool {
    value.get(key) == Some(&Value::Bool(true))
}
fn field<'a>(value: &'a Value, key: &str) -> &'a str {
    value.get(key).and_then(Value::as_str).unwrap_or("")
}

#[cfg(test)]
mod tests;
