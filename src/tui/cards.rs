use super::{
    activity,
    line::Line,
    text,
    theme::{
        BAD, BODY, EMPHASIS, FAINT, GOOD, INSET_SURFACE, MUTED, Style, TOOL_ACCENT,
        TOOL_SELECTED_SURFACE, WARNING,
    },
};
use crate::json::Value;

mod details;
mod diff;
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

// The outcome glyph keeps a fixed column after the tree branch, so outcomes
// read down the left edge. A running call has none yet.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Status {
    Running,
    Background,
    Stopped,
    Failed,
    Done,
}

impl Status {
    fn of(tool: &Tool<'_>) -> Self {
        let Some(result) = tool.result else {
            return Self::Running;
        };
        let process = crate::events::process_status(result);
        if matches!(process, Some("running" | "stopping")) {
            Self::Background
        } else if flag(result, "cancelled") || process == Some("stopped") {
            Self::Stopped
        } else if result.get("error").is_some()
            || flag(result, "timed_out")
            || process == Some("timed_out")
            || matches!(tool.name, "bash" | "process")
                && result
                    .get("exit_code")
                    .is_some_and(|code| code != &Value::number(0))
        {
            Self::Failed
        } else {
            Self::Done
        }
    }
    fn glyph(self) -> &'static str {
        match self {
            Self::Running => "",
            Self::Background => "▶ ",
            Self::Stopped => "■ ",
            Self::Failed => "× ",
            Self::Done => "✓ ",
        }
    }
    fn style(self) -> Style {
        match self {
            Self::Running | Self::Background => TOOL_ACCENT,
            Self::Stopped => WARNING,
            Self::Failed => BAD,
            Self::Done => GOOD,
        }
    }
}

pub fn render(tool: Tool<'_>, columns: usize) -> Vec<Line> {
    let status = Status::of(&tool);
    let width = columns.saturating_sub(5).max(1);
    let continuation = if tool.last { "     " } else { "│    " };
    let (body, position) = output(
        &tool,
        width,
        matches!(status, Status::Stopped | Status::Failed),
    );
    let mut rows = header(&tool, status, columns, position.as_deref())
        .into_iter()
        .map(|row| surface(row, &tool).shortened(columns))
        .collect::<Vec<_>>();
    let mut source_line = 1;
    rows.extend(body.into_iter().map(|mut row| {
        if let Some(origin) = row.origin {
            source_line = source_line.max(origin.line + 1);
        } else {
            row = row.at_source(source_line);
            source_line += 1;
        }
        row.gutter(continuation, FAINT).shortened(columns)
    }));
    rows
}

fn status_line(tool: &Tool<'_>, status: Status, width: usize) -> Line {
    let mut line = Line::new(status.glyph(), status.style());
    line.push(
        &text::ellipsize(&text::clean(&outcome(tool)), (width / 2).max(12)),
        status.style(),
    );
    if let Some(elapsed) = tool.presentation.duration() {
        line.push(
            &format!("  ·  {}", activity::duration(elapsed)),
            status.style(),
        );
    }
    line
}

// The catalog keeps status at the right edge above 72 columns. Narrow cards
// wrap the full target under the name and put status on a separate row.
fn header(tool: &Tool<'_>, status: Status, columns: usize, position: Option<&str>) -> Vec<Line> {
    let guide = if tool.result.is_none() || tool.presentation.selected {
        TOOL_ACCENT
    } else {
        FAINT
    };
    let branch = if tool.last { "└─ " } else { "├─ " };
    let continuation = if tool.last { "     " } else { "│    " };
    let wrapped = if tool.last { "   " } else { "│  " };
    let target_style = if tool.result.is_none() || tool.presentation.selected {
        TOOL_ACCENT
    } else {
        BODY
    };
    let target = match tool.name {
        "bash" => field(tool.arguments, "command").into(),
        "process" => crate::events::description(tool.name, tool.arguments),
        _ => field(tool.arguments, "path").into(),
    };
    let target = text::clean(&target).replace('\n', " ↵ ");
    let mut status_line = status_line(tool, status, columns.saturating_sub(5));
    if let Some(position) = position
        && text::cells(&status_line.plain()) + text::cells(position) + 3
            <= columns.saturating_sub(5)
    {
        status_line.push(&format!(" · {position}"), MUTED);
    }
    let mut command =
        Line::new(&format!("{}  ", text::clean(tool.name)), EMPHASIS.bold()).at_source(0);
    if columns < 72 {
        command.push(&target, target_style);
        let mut rows = command
            .wrap(columns.saturating_sub(3).max(1), false)
            .into_iter()
            .enumerate()
            .map(|(index, line)| {
                line.gutter(if index == 0 { branch } else { wrapped }, guide)
                    .shortened(columns)
            })
            .collect::<Vec<_>>();
        rows.push(status_line.gutter(continuation, FAINT).shortened(columns));
        rows
    } else {
        let mut row = command.gutter(branch, guide);
        let budget = columns
            .saturating_sub(text::cells(&row.plain()) + text::cells(&status_line.plain()) + 3);
        row.push(&text::ellipsize(&target, budget), target_style);
        row.push(
            &" ".repeat(
                columns.saturating_sub(
                    text::cells(&row.plain()) + text::cells(&status_line.plain()) + 1,
                ),
            ),
            MUTED,
        );
        row.spans.extend(status_line.spans);
        vec![row.shortened(columns)]
    }
}

fn output(tool: &Tool<'_>, columns: usize, failed: bool) -> (Vec<Line>, Option<String>) {
    let Some(result) = tool.result else {
        return (Vec::new(), None);
    };
    let (mut rows, position) = match tool.presentation.expanded {
        Some(true) => (
            details::render(tool.name, tool.arguments, result, columns, failed),
            None,
        ),
        Some(false) => (Vec::new(), None),
        None if tool.preview || failed => {
            let preview = preview::render(tool.name, tool.arguments, result, columns, failed);
            let position = if tool.name == "edit" {
                None
            } else {
                preview.position
            };
            (preview.rows, position)
        }
        None => (Vec::new(), None),
    };
    rows.extend(preview::notices(tool.name, result, columns));
    (rows, position)
}

fn surface(mut line: Line, tool: &Tool<'_>) -> Line {
    if tool.presentation.selected {
        line.background = Some(TOOL_SELECTED_SURFACE);
    } else if tool.result.is_none() {
        line.background = Some(INSET_SURFACE);
    }
    line
}
fn outcome(tool: &Tool<'_>) -> String {
    let Some(result) = tool.result else {
        return "running".into();
    };
    if result.get("error").is_some() {
        return "failed".into();
    }
    if let Some(status) = crate::events::process_status(result) {
        return match status {
            "running" => format!("background · {}", field(result, "process_id")),
            "stopping" => format!("stopping · {}", field(result, "process_id")),
            _ => tool.summary.into(),
        };
    }
    match tool.name {
        "bash" if flag(result, "cancelled") => "cancelled".into(),
        "bash" if flag(result, "timed_out") => "timed out".into(),
        "bash" => format!(
            "exit {}",
            result
                .get("exit_code")
                .map_or_else(|| "unknown".into(), Value::encode)
        ),
        "read" => range(tool.arguments, result).unwrap_or_else(|| tool.summary.into()),
        "write" => lines(field(tool.arguments, "content")),
        "edit" => tool.summary.into(),
        _ => tool.summary.into(),
    }
}

/// The returned part of a read: bytes for byte pages, otherwise lines.
fn range(arguments: &Value, result: &Value) -> Option<String> {
    if arguments.get("byte_offset").is_some() {
        let start = result
            .get("byte_offset")
            .and_then(Value::as_usize)
            .unwrap_or(0);
        let count = result
            .get("bytes_returned")
            .and_then(Value::as_usize)
            .unwrap_or(0);
        return Some(if count == 0 {
            "no bytes returned".into()
        } else {
            format!("bytes {start}–{}", start.saturating_add(count - 1))
        });
    }
    let count = result.get("lines_returned").and_then(Value::as_usize)?;
    let start = result
        .get("offset")
        .or_else(|| arguments.get("offset"))
        .and_then(Value::as_usize)
        .unwrap_or(1);
    Some(if count == 0 {
        "no lines returned".into()
    } else {
        format!("lines {start}–{}", start.saturating_add(count - 1))
    })
}

/// A written file's size in the rows its preview counts.
fn lines(content: &str) -> String {
    let content = text::clean(content);
    match content
        .strip_suffix('\n')
        .unwrap_or(&content)
        .split('\n')
        .count()
    {
        _ if content.is_empty() => "empty".into(),
        1 => "1 line".into(),
        count => format!("{count} lines"),
    }
}

fn flag(value: &Value, key: &str) -> bool {
    value.get(key) == Some(&Value::Bool(true))
}
fn field<'a>(value: &'a Value, key: &str) -> &'a str {
    value.get(key).and_then(Value::as_str).unwrap_or("")
}

#[cfg(test)]
mod tests;
