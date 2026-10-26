//! A compact header for related routine tool calls. Individual cards stay in
//! the normal renderer and are shown by the caller when expanded.

use super::{
    Card, Status, Tool, content, field, heading, hint, line_rows, preview, summary, title,
};
use crate::tui::{
    line::Line,
    text,
    theme::{BAD, EMPHASIS, MUTED, Style, TOOL_ACCENT, TOOL_DIM, TOOL_PULSE, WARNING},
};
use std::collections::BTreeSet;

pub fn groupable(tool: &Tool<'_>) -> bool {
    matches!(tool.name, "read" | "bash" | "process" | "web" | "browser")
        && !(tool.name == "read" && field(tool.arguments, "path").starts_with("skill:"))
}

pub fn label<'a>(tool: &Tool<'a>) -> &'a str {
    let work = field(tool.arguments, "work").trim();
    if work.is_empty() { "Activity" } else { work }
}

pub fn header(tools: &[Tool<'_>], columns: usize, keycap: bool) -> Card {
    let mut outcomes = Outcomes::default();
    for tool in tools {
        outcomes.add(Status::of(tool));
    }
    let state = outcomes.primary();
    let marker = if state == Status::Running {
        let elapsed = tools
            .iter()
            .find(|tool| Status::of(tool) == Status::Running)
            .and_then(|tool| tool.presentation.duration())
            .unwrap_or_default();
        TOOL_PULSE[((elapsed.as_millis() / 240) % TOOL_PULSE.len() as u128) as usize]
    } else {
        state.style()
    };

    let mut title = Line::new(
        &text::clean(tools.first().map_or("Activity", label)),
        TOOL_ACCENT.bold(),
    );
    title.push(&format!(" ({}", counted(tools.len(), "call")), MUTED);
    let state_text = match state {
        Status::Failed => format!("{} failed", outcomes.failed),
        Status::Running => "running".into(),
        Status::Background => "background".into(),
        Status::Stopped => "stopped".into(),
        Status::Done => String::new(),
    };
    if !state_text.is_empty() {
        title.push(" · ", MUTED);
        title.push(&state_text, marker);
    }
    title.push(")", MUTED);
    let mut rows = line_rows(&title, columns, 0, "● ", "  ", marker);

    let what = facts(tools, &outcomes);
    rows.extend(line_rows(&what, columns, 1, "  ⎿ ", "    ", TOOL_DIM));

    let hidden = tools.iter().any(|tool| tool.result.is_some())
        && !tools.iter().any(|tool| tool.presentation.expanded);
    if keycap && hidden {
        let last = rows.pop().unwrap_or_default();
        rows.extend(
            hint(last, "expand", columns)
                .into_iter()
                .map(|row| row.shortened(columns)),
        );
    }
    Card { rows, hidden }
}

/// A visible exception under a collapsed chapter, without another status dot.
pub fn exception(tool: &Tool<'_>, columns: usize) -> Vec<Line> {
    let state = Status::of(tool);
    let notices = tool
        .result
        .filter(|_| matches!(tool.name, "bash" | "process"))
        .map_or_else(Vec::new, |result| {
            content::notices(tool.name, result, columns, false)
        });
    if state == Status::Done && notices.is_empty() {
        return Vec::new();
    }
    let (name, target) = heading(tool);
    let mut title = Line::new(name, title::family(name).bold());
    if !target.is_empty() {
        title.push(" ", EMPHASIS);
        title.push(&text::clean(&target), EMPHASIS);
    }
    let mut rows = compact(&title, columns, 2, "  ⎿ ", TOOL_DIM);
    if state == Status::Done {
        rows.extend(notices);
        return rows;
    }

    let result = summary::line(tool, state, None);
    let result = if result.plain().trim().is_empty() {
        Line::new("Running", TOOL_DIM)
    } else {
        result
    };
    let lead = match state {
        Status::Failed => BAD,
        Status::Stopped => WARNING,
        _ => TOOL_DIM,
    };
    rows.extend(compact(&result, columns, 3, "    ", lead));
    rows.extend(notices);
    rows
}

fn compact(line: &Line, columns: usize, source: usize, first: &str, gutter: Style) -> Vec<Line> {
    let line = preview(
        line.clone(),
        columns.saturating_sub(text::cells(first)),
        false,
    );
    let mut rows = line_rows(&line, columns, source, first, "    ", gutter);
    if rows.len() > 2 {
        rows.truncate(2);
        let mut last = rows.pop().unwrap_or_default();
        last.push("…", MUTED);
        rows.push(last.shortened(columns));
    }
    rows
}

#[derive(Default)]
struct Outcomes {
    failed: usize,
    running: usize,
    background: usize,
    stopped: usize,
    done: usize,
}

impl Outcomes {
    fn add(&mut self, status: Status) {
        match status {
            Status::Failed => self.failed += 1,
            Status::Running => self.running += 1,
            Status::Background => self.background += 1,
            Status::Stopped => self.stopped += 1,
            Status::Done => self.done += 1,
        }
    }

    fn primary(&self) -> Status {
        if self.failed > 0 {
            Status::Failed
        } else if self.running > 0 {
            Status::Running
        } else if self.background > 0 {
            Status::Background
        } else if self.stopped > 0 {
            Status::Stopped
        } else {
            Status::Done
        }
    }
}

fn facts(tools: &[Tool<'_>], outcomes: &Outcomes) -> Line {
    let mut files = BTreeSet::new();
    let mut other_reads = 0;
    let mut web = 0;
    let mut screenshots = 0;
    let mut snapshots = 0;
    let mut browser = 0;
    let mut commands = 0;
    let mut processes = 0;
    for tool in tools {
        if Status::of(tool) != Status::Done {
            continue;
        }
        match tool.name {
            "read" => {
                let path = field(tool.arguments, "path");
                if ["history:", "output:", "attachment:", "checkpoint:"]
                    .iter()
                    .any(|prefix| path.starts_with(*prefix))
                    || path.is_empty()
                {
                    other_reads += 1;
                } else if !path.starts_with("skill:") {
                    files.insert(path);
                }
            }
            "bash" => commands += 1,
            "process" => processes += 1,
            "web" => web += 1,
            "browser" => {
                if field(tool.arguments, "action") == "screenshot" {
                    screenshots += 1;
                } else if field(tool.arguments, "action") == "snapshot" {
                    snapshots += 1;
                } else {
                    browser += 1;
                }
            }
            _ => {}
        }
    }
    let mut parts = Vec::new();
    if !files.is_empty() {
        parts.push(format!(
            "{} file{} read",
            files.len(),
            if files.len() == 1 { "" } else { "s" }
        ));
    }
    for (count, noun) in [
        (other_reads, "other read"),
        (web, "web call"),
        (screenshots, "screenshot"),
        (snapshots, "snapshot"),
        (browser, "browser action"),
        (commands, "command"),
        (processes, "process call"),
    ] {
        if count > 0 {
            let completed = matches!(noun, "web call" | "command" | "process call");
            parts.push(format!(
                "{}{}",
                counted(count, noun),
                if completed { " completed" } else { "" }
            ));
        }
    }
    let mut line = Line::new(&parts.join(" · "), MUTED);
    for (count, label, style) in [
        (outcomes.failed, "failed", BAD),
        (outcomes.stopped, "stopped", WARNING),
        (outcomes.running, "running", TOOL_ACCENT),
        (outcomes.background, "in background", TOOL_ACCENT),
    ] {
        if count > 0 {
            if !line.spans.is_empty() {
                line.push(" · ", MUTED);
            }
            line.push(&format!("{count} {label}"), style);
        }
    }
    if line.spans.is_empty() {
        Line::new(
            &format!("{} completed", counted(outcomes.done, "call")),
            MUTED,
        )
    } else {
        line
    }
}

fn counted(count: usize, noun: &str) -> String {
    format!("{count} {noun}{}", if count == 1 { "" } else { "s" })
}
