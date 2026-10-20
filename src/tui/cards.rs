use super::{
    activity,
    line::Line,
    text,
    theme::{BAD, EMPHASIS, GOOD, MUTED, Style, TOOL_ACCENT, TOOL_DIM, WARNING},
};
use crate::{file_change::Change, json::Value};

mod content;
mod diff;
mod presentation;
pub use presentation::Presentation;

pub struct Tool<'a> {
    pub name: &'a str,
    pub arguments: &'a Value,
    pub summary: &'a str,
    pub result: Option<&'a Value>,
    pub change: Option<&'a Change>,
    pub presentation: &'a Presentation,
}

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
                    .is_some_and(|code| code != &Value::number(0) && code != &Value::Null)
        {
            Self::Failed
        } else {
            Self::Done
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

pub(super) struct Body {
    rows: Vec<Line>,
    hidden: usize,
    unit: &'static str,
}

impl Body {
    fn new(rows: Vec<Line>) -> Self {
        Self {
            rows,
            hidden: 0,
            unit: "output lines",
        }
    }
}

pub fn render(tool: Tool<'_>, columns: usize) -> Vec<Line> {
    let status = Status::of(&tool);
    let expanded = tool.presentation.expanded == Some(true);
    // Old conversations retain replacement arguments but have no file snapshot.
    // Their coordinates stay absent rather than being reconstructed from disk.
    let legacy = (tool.change.is_none() && tool.name == "edit" && status == Status::Done)
        .then(|| {
            let mut change = Change::between(
                Some(field(tool.arguments, "old_text")),
                field(tool.arguments, "new_text"),
            )?;
            for entry in change.hunks.iter_mut().flatten() {
                entry.old_line = None;
                entry.new_line = None;
                entry.no_newline = false;
            }
            change.ending_change = None;
            change.final_newline = None;
            Some(change)
        })
        .flatten();
    let change = tool
        .change
        .or(legacy.as_ref())
        .filter(|_| status == Status::Done);
    let mut rows = header(&tool, status, columns);
    let summary = preview(
        Line::new(&outcome(&tool, change), status_summary(status)),
        columns.saturating_sub(4),
        expanded,
    )
    .plain();
    rows.extend(wrapped(
        &summary,
        status_summary(status),
        columns,
        1,
        "  ⎿ ",
        "    ",
        true,
    ));
    if let Some(result) = tool.result {
        let mut body = if result.get("error").is_some() {
            Body::new(Vec::new())
        } else if let Some(change) = change {
            diff::render(change, field(tool.arguments, "path"), columns, expanded)
        } else {
            content::render(
                tool.name,
                tool.arguments,
                result,
                columns,
                status == Status::Failed,
                expanded,
            )
        };
        if body.hidden > 0 {
            let origin = body
                .rows
                .iter()
                .filter_map(|line| line.origin)
                .map(|origin| origin.line)
                .max()
                .unwrap_or(2)
                + 1;
            let unit = if body.hidden == 1 {
                body.unit.strip_suffix('s').unwrap_or(body.unit)
            } else {
                body.unit
            };
            body.rows.extend(wrapped(
                &format!("… {} more {unit}", body.hidden),
                MUTED,
                columns,
                origin,
                "    ",
                "    ",
                true,
            ));
        }
        rows.extend(body.rows);
        rows.extend(content::notices(tool.name, result, columns));
    }
    if expanded && let Some(elapsed) = tool.presentation.duration() {
        rows.extend(wrapped(
            &format!("Elapsed {}", activity::duration(elapsed)),
            TOOL_DIM,
            columns,
            usize::MAX - 1,
            "    ",
            "    ",
            true,
        ));
    }
    rows
}

fn status_summary(status: Status) -> Style {
    match status {
        Status::Failed | Status::Stopped => status.style(),
        _ => MUTED,
    }
}

fn header(tool: &Tool<'_>, status: Status, columns: usize) -> Vec<Line> {
    let skill = (tool.name == "read")
        .then(|| skill_name(field(tool.arguments, "path")))
        .flatten();
    let name = match tool.name {
        "read" if skill.is_some() => "Skill",
        "read" => "Read",
        "write" => "Write",
        "edit" => "Update",
        "bash" => "Bash",
        "process" => "Process",
        other => other,
    };
    let target = match tool.name {
        "bash" => field(tool.arguments, "command").into(),
        "process" => crate::events::description(tool.name, tool.arguments),
        _ => skill.unwrap_or(field(tool.arguments, "path")).into(),
    };
    let mut title = Line::new(&format!("{name}("), EMPHASIS.bold()).at_source(0);
    title.push(&text::clean(&target).replace('\n', " ↵ "), EMPHASIS);
    title.push(")", EMPHASIS.bold());
    preview(
        title,
        columns.saturating_sub(2),
        tool.presentation.expanded == Some(true),
    )
    .wrap(columns.saturating_sub(2).max(1), false)
    .into_iter()
    .enumerate()
    .map(|(index, row)| {
        row.gutter(if index == 0 { "● " } else { "  " }, status.style())
            .shortened(columns)
    })
    .collect()
}

fn outcome(tool: &Tool<'_>, change: Option<&Change>) -> String {
    let Some(result) = tool.result else {
        return tool.presentation.duration().map_or_else(
            || "Running…".into(),
            |elapsed| format!("Running… · {}", activity::duration(elapsed)),
        );
    };
    if let Some(error) = result.get("error").and_then(Value::as_str) {
        return format!("Failed · {}", text::clean(error));
    }
    if flag(result, "cancelled") {
        return "Cancelled".into();
    }
    if flag(result, "timed_out") || crate::events::process_status(result) == Some("timed_out") {
        return tool.presentation.duration().map_or_else(
            || "Timed out".into(),
            |elapsed| format!("Timed out after {}", activity::duration(elapsed)),
        );
    }
    if let Some(processes) = result.get("processes").and_then(Value::as_array) {
        let running = processes
            .iter()
            .filter(|item| crate::events::process_status(item) == Some("running"))
            .count();
        return format!(
            "{} {} · {running} running",
            processes.len(),
            if processes.len() == 1 {
                "process"
            } else {
                "processes"
            }
        );
    }
    if let Some(status) = crate::events::process_status(result) {
        let id = field(result, "process_id");
        match status {
            "running" if tool.name == "bash" => {
                return format!("Started in background · {id}");
            }
            "running" => return format!("Running at last check · {id}"),
            "stopping" => return format!("Stopping · {id}"),
            "stopped" => return format!("Stopped · {id}"),
            "unknown" => return format!("Status unknown · {id}"),
            _ => {}
        }
    }
    if let Some(change) = change {
        return diff::summary(change);
    }
    match tool.name {
        "bash" | "process" => {
            let code = result
                .get("exit_code")
                .filter(|value| **value != Value::Null)
                .map_or_else(|| "unknown".into(), Value::encode);
            format!(
                "{}Exit {code}",
                if Status::of(tool) == Status::Failed {
                    "Failed · "
                } else {
                    ""
                }
            )
        }
        "read" if skill_name(field(tool.arguments, "path")).is_some() => {
            format!("Loaded skill · {}", read_summary(tool.arguments, result))
        }
        "read" => read_summary(tool.arguments, result),
        "write" => {
            let count = content::payload(field(tool.arguments, "content")).len();
            format!(
                "Wrote {count} {} · previous content unavailable",
                plural(count)
            )
        }
        _ => tool.summary.into(),
    }
}

fn skill_name(path: &str) -> Option<&str> {
    let name = path.strip_prefix("skill:")?;
    let name = name.strip_suffix("/SKILL.md").unwrap_or(name);
    (!name.is_empty() && !name.contains('/')).then_some(name)
}

fn read_summary(arguments: &Value, result: &Value) -> String {
    if let Some(view) = result.get("view").and_then(Value::as_str) {
        return format!("Opened {}", if view == "pdf" { "PDF" } else { view });
    }
    if result.get("attachment").is_some() && result.get("content").is_none() {
        return "Attachment available".into();
    }
    if arguments.get("byte_offset").is_some() || result.get("offset") == Some(&Value::Null) {
        let count = result
            .get("bytes_returned")
            .and_then(Value::as_usize)
            .unwrap_or(0);
        let start = result
            .get("byte_offset")
            .and_then(Value::as_usize)
            .unwrap_or(0);
        return if count == 0 {
            "Read 0 bytes".into()
        } else {
            format!(
                "Read {count} bytes · bytes {start}–{}",
                start.saturating_add(count - 1)
            )
        };
    }
    let count = result
        .get("lines_returned")
        .and_then(Value::as_usize)
        .unwrap_or_else(|| content::payload(field(result, "content")).len());
    let start = result
        .get("offset")
        .or_else(|| arguments.get("offset"))
        .and_then(Value::as_usize)
        .unwrap_or(1);
    if count == 0 {
        "Read 0 lines".into()
    } else {
        format!(
            "Read {count} {} · lines {start}–{}",
            plural(count),
            start.saturating_add(count - 1)
        )
    }
}

fn plural(count: usize) -> &'static str {
    if count == 1 { "line" } else { "lines" }
}

fn preview(line: Line, width: usize, expanded: bool) -> Line {
    if expanded {
        line
    } else {
        // A wide glyph can leave one unused cell at each wrap boundary.
        line.shortened(width.saturating_sub(1).max(1).saturating_mul(3))
    }
}

fn wrapped(
    value: &str,
    style: Style,
    columns: usize,
    source: usize,
    first: &str,
    rest: &str,
    words: bool,
) -> Vec<Line> {
    Line::new(value, style)
        .at_source(source)
        .wrap(columns.saturating_sub(text::cells(first)).max(1), words)
        .into_iter()
        .enumerate()
        .map(|(index, row)| {
            row.gutter(if index == 0 { first } else { rest }, MUTED)
                .shortened(columns)
        })
        .collect()
}

fn flag(value: &Value, key: &str) -> bool {
    value.get(key) == Some(&Value::Bool(true))
}
fn field<'a>(value: &'a Value, key: &str) -> &'a str {
    value.get(key).and_then(Value::as_str).unwrap_or("")
}

#[cfg(test)]
mod skill_tests;
#[cfg(test)]
mod tests;
