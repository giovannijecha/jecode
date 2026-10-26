//! A compact header for related routine tool calls. Individual cards stay in
//! the normal renderer and are shown by the caller when expanded.

use super::{Card, Status, Tool, content, failure, field, heading, rhythm, summary, title};
use crate::tui::{
    activity as timer,
    line::{Line, Span},
    text,
    theme::{BODY, MUTED, SELECTED_SURFACE, STRONG, TOOL_ACCENT, TOOL_DIM, TOOL_PULSE},
};
use std::collections::BTreeSet;

pub fn groupable(tool: &Tool<'_>) -> bool {
    matches!(tool.name, "read" | "bash" | "process" | "web" | "browser")
        && !(tool.name == "read" && field(tool.arguments, "path").starts_with("skill:"))
}

pub fn label(tool: &Tool<'_>) -> String {
    let work = field(tool.arguments, "work")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if work.is_empty() {
        "Activity".into()
    } else {
        text::clean(&work)
    }
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

    let label = tools.first().map_or_else(|| "Activity".into(), label);
    let mut title = Line::new(&text::ellipsize(&label, 60), TOOL_ACCENT.bold());
    title.push(&format!("  {}", counted(tools.len(), "call")), TOOL_DIM);
    let state_text = match state {
        Status::Failed => format!("{} failed", outcomes.failed),
        Status::Running => "running".into(),
        Status::Background => "background".into(),
        Status::Stopped => "stopped".into(),
        Status::Done => String::new(),
    };
    if !state_text.is_empty() {
        title.push(" · ", TOOL_DIM);
        title.push(&state_text, marker);
    }
    let hidden = tools.iter().any(|tool| tool.result.is_some())
        && !tools.iter().any(|tool| tool.presentation.expanded);
    let title = if keycap && hidden {
        title_with_keycap(title, columns, marker)
    } else {
        rhythm::one(title, columns, "● ", marker, 0)
    };
    let mut rows = vec![title];

    rows.push(rhythm::one(
        facts(tools, &outcomes),
        columns,
        "  ⎿ ",
        TOOL_DIM,
        1,
    ));

    Card { rows, hidden }
}

fn title_with_keycap(title: Line, columns: usize, marker: crate::tui::theme::Style) -> Line {
    let mut cap = Line::default();
    cap.spans.push(Span {
        text: " Ctrl+O ".into(),
        style: STRONG,
        background: Some(SELECTED_SURFACE),
    });
    if columns >= 19 {
        cap.push(" expand", MUTED);
    }
    let cap_width = text::cells(&cap.plain());
    let left_width = columns.saturating_sub(cap_width + 1);
    let mut line = rhythm::one(title, left_width, "● ", marker, 0);
    let gap = columns.saturating_sub(text::cells(&line.plain()) + cap_width);
    line.push(&" ".repeat(gap), MUTED);
    line.append(cap).shortened(columns)
}

/// The latest ordinary call without separate error detail stays at the foot.
pub fn activity(tools: &[Tool<'_>], columns: usize) -> Option<Line> {
    let tool = tools.iter().rev().find(|tool| {
        matches!(tool.name, "read" | "web" | "browser") && Status::of(tool) != Status::Failed
    })?;
    let (name, target) = heading(tool);
    let mut line = Line::new(name, title::family(name).bold());
    if !target.is_empty() {
        line.push(&format!("  {}", text::clean(&target)), BODY);
    }
    let status = Status::of(tool);
    let elapsed = tool
        .presentation
        .duration()
        .or_else(|| (tool.result.is_none()).then(Default::default));
    let mut suffix = String::new();
    if let Some(elapsed) = elapsed {
        suffix.push_str(&format!("  {}", timer::duration(elapsed)));
    }
    if matches!(
        status,
        Status::Failed | Status::Stopped | Status::Background
    ) {
        let state = match status {
            Status::Failed => "failed",
            Status::Stopped => "stopped",
            Status::Background => "background",
            _ => unreachable!(),
        };
        suffix.push_str(&format!("  {state}"));
    }
    if !suffix.is_empty() {
        line = line.shortened(columns.saturating_sub(4 + text::cells(&suffix)));
        line.push(&suffix, TOOL_DIM);
    }
    Some(rhythm::one(line, columns, "  ⎿ ", TOOL_DIM, 4))
}

/// Facts retained from the former singleton while a new chapter takes shape.
/// The caller uses only as many rows as its content height requires.
pub fn receipts(tool: &Tool<'_>, columns: usize) -> Vec<Line> {
    let Some(result) = tool.result else {
        return activity(std::slice::from_ref(tool), columns)
            .into_iter()
            .collect();
    };
    let mut rows = Vec::new();
    if matches!(tool.name, "bash" | "process") {
        rows.extend(content::notices(tool.name, result, columns, false));
        let outcome = match Status::of(tool) {
            // Exit status already appears on the member. Keep a distinct
            // error fact, or let the captured-output count supply the receipt.
            Status::Failed => field(result, "stderr")
                .lines()
                .map(str::trim)
                .find(|line| !line.is_empty())
                .map(text::clean),
            Status::Done => Some(result.get("exit_code").map_or_else(
                || "Command completed".into(),
                |code| format!("exit {}", code.encode()),
            )),
            Status::Background => Some("Process in background".into()),
            Status::Stopped => Some("Process stopped".into()),
            Status::Running => Some("Command running".into()),
        };
        if let Some(outcome) = outcome {
            rows.push(rhythm::one(
                Line::new(&outcome, TOOL_DIM),
                columns,
                "  ⎿ ",
                TOOL_DIM,
                3,
            ));
        }
        let output_lines = [field(result, "stderr"), field(result, "stdout")]
            .into_iter()
            .flat_map(str::lines)
            .filter(|line| !line.trim().is_empty())
            .count();
        if output_lines > 0 {
            rows.push(rhythm::one(
                Line::new(&format!("{output_lines} output lines captured"), TOOL_DIM),
                columns,
                "  ⎿ ",
                TOOL_DIM,
                4,
            ));
        }
        let command = result
            .get("command")
            .and_then(crate::json::Value::as_str)
            .filter(|command| !command.is_empty())
            .unwrap_or_else(|| field(tool.arguments, "command"));
        if !rhythm::description(tool).is_empty() && !command.is_empty() {
            rows.push(rhythm::one(
                Line::new(&format!("Command: {}", text::clean(command)), TOOL_DIM),
                columns,
                "  ⎿ ",
                TOOL_DIM,
                5,
            ));
        }
    } else {
        rows.push(rhythm::one(
            summary::line(tool, Status::of(tool), None),
            columns,
            "  ⎿ ",
            TOOL_DIM,
            3,
        ));
        rows.extend(content::notices(tool.name, result, columns, false));
    }
    rows
}

/// One primary row for commands and exceptional calls; errors add detail below.
pub fn exception(tool: &Tool<'_>, columns: usize) -> Vec<Line> {
    let state = Status::of(tool);
    if matches!(tool.name, "bash" | "process") {
        let (name, _) = heading(tool);
        let mut line = Line::new(name, title::family(name).bold());
        line.push("  ", BODY);
        let description = rhythm::description(tool);
        let target = if !description.is_empty() {
            text::clean(description)
        } else if tool.name == "process" {
            let command = tool
                .result
                .map(|result| field(result, "command"))
                .filter(|command| !command.is_empty())
                .unwrap_or_else(|| field(tool.arguments, "command"));
            if command.is_empty() {
                field(tool.arguments, "action").into()
            } else {
                text::clean(command)
            }
        } else {
            rhythm::payload(tool)
        };
        line.push(&target, BODY);
        let detail = match state {
            Status::Running => Some(format!(
                "running · {}",
                timer::duration(tool.presentation.duration().unwrap_or_default())
            )),
            Status::Done => tool.presentation.duration().map(timer::duration),
            Status::Failed => Some(
                tool.result
                    .and_then(|result| result.get("exit_code"))
                    .filter(|code| **code != crate::json::Value::Null)
                    .map_or_else(|| "failed".into(), |code| format!("exit {}", code.encode())),
            ),
            Status::Background => Some("background".into()),
            Status::Stopped => Some("stopped".into()),
        };
        if let Some(detail) = detail {
            let suffix = format!("  {detail}");
            line = line.shortened(columns.saturating_sub(4 + text::cells(&suffix)));
            line.push(&suffix, TOOL_DIM);
        }
        return vec![rhythm::one(line, columns, "  ⎿ ", TOOL_DIM, 2)];
    }
    if matches!(tool.name, "read" | "web" | "browser")
        && matches!(state, Status::Running | Status::Done)
    {
        return Vec::new();
    }
    let (name, _) = heading(tool);
    let mut title = Line::new(name, title::family(name).bold());
    if tool.name == "browser" {
        title.push(&format!(" {}", field(tool.arguments, "action")), MUTED);
    }
    title.push("  ", BODY);
    let description = rhythm::description(tool);
    if !description.is_empty() {
        title.push(&text::clean(description), BODY);
    } else if matches!(tool.name, "bash" | "process" | "browser") {
        title = title.append(rhythm::code(tool));
    } else {
        title.push(&rhythm::payload(tool), BODY);
    }
    let detail = match state {
        Status::Running => Some(format!(
            "Running · {}",
            timer::duration(tool.presentation.duration().unwrap_or_default())
        )),
        Status::Background | Status::Stopped => Some(summary::line(tool, state, None).plain()),
        Status::Failed | Status::Done => None,
    };
    if let Some(detail) = detail {
        let suffix = format!("  {detail}");
        title = title.shortened(columns.saturating_sub(4 + text::cells(&suffix)));
        title.push(&suffix, TOOL_DIM);
    }
    let mut rows = vec![rhythm::one(title, columns, "  ⎿ ", TOOL_DIM, 2)];
    if state == Status::Failed {
        rows.extend(failure::rows(tool, columns, "    "));
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
            parts.push(counted(count, noun));
        }
    }
    let mut line = Line::new(&parts.join(" · "), TOOL_DIM);
    for (count, label, style) in [
        (outcomes.stopped, "stopped", TOOL_DIM),
        (outcomes.running, "running", TOOL_DIM),
        (outcomes.background, "in background", TOOL_DIM),
    ] {
        if count > 0 {
            if !line.spans.is_empty() {
                line.push(" · ", TOOL_DIM);
            }
            line.push(&format!("{count} {label}"), style);
        }
    }
    if line.spans.is_empty() {
        Line::new(&format!("{} completed", outcomes.done), TOOL_DIM)
    } else {
        line
    }
}

fn counted(count: usize, noun: &str) -> String {
    format!("{count} {noun}{}", if count == 1 { "" } else { "s" })
}
