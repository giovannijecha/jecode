use super::{
    line::{Line, Span},
    text,
    theme::{DIM, MUTED, SELECTED_SURFACE, STRONG, Style},
};
use crate::{file_change::Change, json::Value};

mod agents;
mod badge;
mod chapter;
mod content;
mod diff;
mod discovery;
mod edit_fallback;
mod failure;
mod map;
mod presentation;
mod rhythm;
mod streams;
mod summary;
mod title;
mod work;
pub(in crate::tui) use badge::hang;
pub use chapter::{
    activities as chapter_activities, exception as chapter_exception,
    groupable as chapter_groupable, header as chapter_header, label as chapter_label,
    receipts as chapter_receipts,
};
pub use presentation::Presentation;

#[derive(Clone, Copy)]
pub struct Tool<'a> {
    pub name: &'a str,
    pub arguments: &'a Value,
    pub summary: &'a str,
    pub result: Option<&'a Value>,
    pub change: Option<&'a Change>,
    pub presentation: &'a Presentation,
}

/// A card's rows, and whether Ctrl+O would show more of it.
pub struct Card {
    pub rows: Vec<Line>,
    pub hidden: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Status {
    Running,
    Background,
    Stopped,
    Failed,
    /// A collaborator's outcome is not confirmed, such as after a restart.
    Unknown,
    Done,
}

impl Status {
    fn of(tool: &Tool<'_>) -> Self {
        let Some(result) = tool.result else {
            return Self::Running;
        };
        if tool.name == "collaboration" {
            return agents::status(result);
        }
        let process = crate::events::process_status(result);
        if matches!(process, Some("running" | "stopping")) {
            Self::Background
        } else if flag(result, "cancelled")
            || flag(result, "timed_out")
            || matches!(process, Some("stopped" | "timed_out"))
        {
            Self::Stopped
        } else if result
            .get("error")
            .is_some_and(|error| error != &Value::Null)
            || matches!(tool.name, "bash" | "exec" | "process")
                && result
                    .get("exit_code")
                    .is_some_and(|code| code != &Value::number(0) && code != &Value::Null)
        {
            Self::Failed
        } else {
            Self::Done
        }
    }
}

pub(super) struct Body {
    rows: Vec<Line>,
    hidden: usize,
    hidden_hunks: usize,
    unit: &'static str,
}

impl Body {
    fn new(rows: Vec<Line>) -> Self {
        Self {
            rows,
            hidden: 0,
            hidden_hunks: 0,
            unit: "lines",
        }
    }
}

#[cfg(test)]
pub fn render(tool: Tool<'_>, columns: usize) -> Vec<Line> {
    card(tool, columns, false).rows
}

/// The `/agents` name badge for a collaborator in `state`: shimmering while
/// it works, red after a failure, amber when stopped, lit on the quiet
/// surface when unknown.
pub(in crate::tui) fn agent_badge(name: &str, state: &str) -> Line {
    badge::badge(
        name,
        match state {
            "running" | "stopping" => Status::Running,
            "failed" => Status::Failed,
            "stopped" => Status::Stopped,
            "unknown" => Status::Background,
            _ => Status::Done,
        },
    )
}

/// One card. `keycap` names Ctrl+O on the row that counts hidden lines.
pub fn card(tool: Tool<'_>, columns: usize, keycap: bool) -> Card {
    let mut card = if tool.name == "ask_user" {
        work::ask(tool, columns)
    } else if tool.name == "todo_list" {
        work::todo(tool, columns)
    } else if tool.name == "collaboration" {
        agents::card(tool, columns, keycap)
    } else if matches!(tool.name, "read" | "write" | "edit") {
        file_card(tool, columns, keycap)
    } else {
        rhythm::card(tool, columns, keycap)
    };
    // An expanded turn names the way back on its first title row.
    if keycap && tool.presentation.expanded && !card.rows.is_empty() {
        let title = card.rows.remove(0);
        card.rows.splice(0..0, hint(title, "collapse", columns));
    }
    card
}

fn file_card(tool: Tool<'_>, columns: usize, keycap: bool) -> Card {
    let status = Status::of(&tool);
    let expanded = tool.presentation.expanded;
    // Diff limits leave only the supplied replacements; file coordinates
    // cannot be recovered from those snippets.
    let legacy = (tool.change.is_none() && tool.name == "edit" && status == Status::Done)
        .then(|| edit_fallback::single_change(tool.arguments))
        .flatten();
    let change = tool
        .change
        .or(legacy.as_ref())
        .filter(|_| status == Status::Done);
    let (name, target) = heading(&tool);
    let mut rows = title::rows(name, &target, status, columns, expanded);
    let titled = rows.len();
    let summary = summary::line(&tool, status, change);
    let fact = if tool.name == "read" && !expanded {
        summary::compact_read(summary, columns.saturating_sub(4))
    } else {
        preview(summary, columns.saturating_sub(4), expanded)
    };
    // The facts join a one-row title when they fit; otherwise they hang.
    match rows.as_mut_slice() {
        [title] if text::cells(&title.plain()) + 2 + text::cells(&fact.plain()) <= columns => {
            title.push("  ", MUTED);
            title.spans.extend(fact.spans);
        }
        _ => rows.extend(line_rows(&fact, columns, 1, "  ⎿ ", "    ", DIM)),
    }
    let facts = rows.len();
    let mut hidden = false;
    let mut diff = false;
    if let Some(result) = tool.result {
        let mut body =
            if result.get("error").is_some() || (tool.name == "edit" && status != Status::Done) {
                Body::new(Vec::new())
            } else if let Some(change) = change.filter(|change| change.created) {
                content::created(change, field(tool.arguments, "path"), columns, expanded)
            } else if let Some(change) = change {
                diff = true;
                diff::render(change, field(tool.arguments, "path"), columns, expanded)
            } else {
                content::render(&tool, result, status, columns, expanded)
            };
        if body.hidden > 0 {
            hidden = true;
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
            let label = if body.hidden_hunks > 0 {
                format!(
                    "+{} {} · {} rows",
                    body.hidden_hunks,
                    if body.hidden_hunks == 1 {
                        "hunk"
                    } else {
                        "hunks"
                    },
                    body.hidden
                )
            } else {
                format!("+{} {unit}", body.hidden)
            };
            // A diff runs from column 2, so its count does too.
            let gutter = if diff { "  " } else { "    " };
            let mut more = line_rows(
                &Line::new(&label, DIM),
                columns,
                origin,
                gutter,
                gutter,
                DIM,
            );
            if keycap && let Some(last) = more.pop() {
                more.extend(hint(last, "expand", columns));
            }
            body.rows.extend(more);
        }
        rows.extend(body.rows);
        rows.extend(content::notices(tool.name, result, columns, expanded));
    }
    // A diff runs from column 2 below its facts, outside the tree.
    let hung = if diff { facts } else { rows.len() };
    badge::hang(&mut rows[titled..hung], true);
    Card { rows, hidden }
}

fn heading<'a>(tool: &Tool<'a>) -> (&'a str, String) {
    let skill = (tool.name == "read")
        .then(|| skill_name(field(tool.arguments, "path")))
        .flatten();
    let name = match tool.name {
        "read" if skill.is_some() => "Skill",
        "read" => "Read",
        "list" => "List",
        "search" => "Search",
        "map" => "Map",
        "write" => "Write",
        "edit" => "Update",
        "bash" => "Bash",
        "exec" => "Exec",
        "process" => "Process",
        "checkpoint" => "Checkpoint",
        "web" => "Web",
        "browser" => "Browser",
        "agent" => "Agents",
        "collaboration" => field(tool.arguments, "label"),
        other => other,
    };
    let target = match tool.name {
        "bash" => field(tool.arguments, "command").into(),
        "list" | "search" | "map" | "exec" | "process" | "checkpoint" | "web" | "browser"
        | "agent" => crate::events::target(tool.name, tool.arguments),
        _ => skill.unwrap_or(field(tool.arguments, "path")).into(),
    };
    (name, target)
}

/// `row` with a right-aligned Ctrl+O keycap, or the keycap on its own row.
fn hint(row: Line, label: &str, columns: usize) -> Vec<Line> {
    let mut cap = Line::default();
    cap.spans.push(Span {
        text: " Ctrl+O ".into(),
        style: STRONG,
        background: Some(SELECTED_SURFACE),
    });
    cap.push(&format!(" {label}"), DIM);
    let used = text::cells(&row.plain());
    let room = text::cells(&cap.plain());
    if used + 2 + room <= columns {
        let mut row = row;
        row.push(&" ".repeat(columns - used - room), MUTED);
        vec![row.append(cap)]
    } else {
        let origin = row.origin;
        let mut cap = cap.indent("    ", MUTED);
        cap.origin = origin;
        vec![row, cap]
    }
}

fn skill_name(path: &str) -> Option<&str> {
    let name = path.strip_prefix("skill:")?;
    let name = name.strip_suffix("/SKILL.md").unwrap_or(name);
    (!name.is_empty() && !name.contains('/')).then_some(name)
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

/// `line` word-wrapped under `first`, then `rest`, in the `gutter` style.
fn line_rows(
    line: &Line,
    columns: usize,
    source: usize,
    first: &str,
    rest: &str,
    gutter: Style,
) -> Vec<Line> {
    line.clone()
        .at_source(source)
        .wrap(columns.saturating_sub(text::cells(first)).max(1), true)
        .into_iter()
        .enumerate()
        .map(|(index, row)| {
            row.gutter(if index == 0 { first } else { rest }, gutter)
                .shortened(columns)
        })
        .collect()
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
            row.gutter(if index == 0 { first } else { rest }, DIM)
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
mod chapter_tests;
#[cfg(test)]
mod checkpoint_tests;
#[cfg(test)]
mod discovery_tests;
#[cfg(test)]
mod edit_fallback_tests;
#[cfg(test)]
mod files_tests;
#[cfg(test)]
mod map_tests;
#[cfg(test)]
mod rhythm_tests;
#[cfg(test)]
mod skill_tests;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod typed_tests;
