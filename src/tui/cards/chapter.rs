//! A compact header for related routine tool calls. Individual cards stay in
//! the normal renderer and are shown by the caller when expanded.

use super::{Card, Status, Tool, agents, badge, content, failure, field, heading, rhythm, summary};
use crate::tui::{
    activity as timer,
    line::{Line, Span},
    text,
    theme::{BAD, BODY, DIM, MUTED, SELECTED_SURFACE, STRONG},
};
mod facts;
use facts::{Outcomes, facts};

pub fn groupable(tool: &Tool<'_>) -> bool {
    // Agent events and inspection join chapters; a lone event and the
    // actions that change work, such as integrate, stand alone.
    match tool.name {
        "collaboration" => {
            return tool.arguments.get("lone") != Some(&crate::json::Value::Bool(true));
        }
        "agent" => {
            return matches!(
                field(tool.arguments, "action"),
                "list" | "inspect" | "wait" | "diff"
            );
        }
        _ => {}
    }
    matches!(
        tool.name,
        "read" | "list" | "search" | "map" | "bash" | "exec" | "process" | "web" | "browser"
    ) && !(tool.name == "read" && field(tool.arguments, "path").starts_with("skill:"))
        && (tool.name != "map" || matches!(field(tool.arguments, "action"), "" | "overview"))
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
    // The WORK badge carries the chapter's state; only failures are counted.
    let mut title = badge::badge("Work", outcomes.primary());
    title.push(" ", BODY);
    let label = tools.first().map_or_else(|| "Activity".into(), label);
    title.push(&text::ellipsize(&label, 60), STRONG.bold());
    title.push(&format!("  {}", text::counted(tools.len(), "call")), DIM);
    if outcomes.failed > 0 {
        title.push(&format!(" · {} failed", outcomes.failed), BAD);
    }
    let expanded = tools.iter().any(|tool| tool.presentation.expanded);
    let hidden = tools.iter().any(|tool| tool.result.is_some()) && !expanded;
    let title = if keycap && (hidden || expanded) {
        title_with_keycap(title, columns, if expanded { "collapse" } else { "expand" })
    } else {
        title.shortened(columns)
    };
    let mut rows = vec![title.at_source(0)];
    rows.push(rhythm::one(
        facts(tools, &outcomes),
        columns,
        "  ⎿ ",
        DIM,
        1,
    ));
    // The tree stays open; the caller closes it on the chapter's last row.
    badge::hang(&mut rows[1..], false);

    Card { rows, hidden }
}

fn title_with_keycap(title: Line, columns: usize, action: &str) -> Line {
    let mut cap = Line::default();
    cap.spans.push(Span {
        text: " Ctrl+O ".into(),
        style: STRONG,
        background: Some(SELECTED_SURFACE),
    });
    if columns >= 19 {
        cap.push(&format!(" {action}"), DIM);
    }
    let cap_width = text::cells(&cap.plain());
    let left_width = columns.saturating_sub(cap_width + 1);
    let mut line = title.shortened(left_width);
    let gap = columns.saturating_sub(text::cells(&line.plain()) + cap_width);
    line.push(&" ".repeat(gap), MUTED);
    line.append(cap).shortened(columns)
}

/// A member's name in its family colour, uppercase like its badge.
fn named(tool: &Tool<'_>) -> Line {
    let (name, _) = heading(tool);
    Line::new(&name.to_uppercase(), badge::ink(name).bold())
}

fn ordinary(tool: &Tool<'_>) -> bool {
    matches!(
        tool.name,
        "read" | "list" | "search" | "map" | "web" | "browser"
    )
}

/// Keep the latest completed targets at the foot, up to the call concurrency
/// limit. A running target takes one of those rows, so finishing it cannot
/// shorten a live chapter or move earlier transcript down.
pub fn activities(tools: &[Tool<'_>], columns: usize) -> Vec<Line> {
    const VISIBLE: usize = 4;
    let running = tools
        .iter()
        .filter(|tool| ordinary(tool) && Status::of(tool) == Status::Running)
        .count();
    let keep = VISIBLE.saturating_sub(running);
    let mut completed = tools
        .iter()
        .filter(|tool| {
            ordinary(tool) && matches!(Status::of(tool), Status::Done | Status::Background)
        })
        .rev()
        .take(keep)
        .collect::<Vec<_>>();
    completed.reverse();
    completed
        .into_iter()
        .map(|tool| activity_line(tool, columns))
        .collect()
}

/// The last ordinary activity, also used as a singleton's retained receipt.
pub fn activity(tools: &[Tool<'_>], columns: usize) -> Option<Line> {
    activities(tools, columns).pop()
}

fn activity_line(tool: &Tool<'_>, columns: usize) -> Line {
    let (_, target) = heading(tool);
    let mut line = named(tool);
    if !target.is_empty() {
        line.push(&format!(" {}", text::clean(&target)), BODY);
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
        line.push(&suffix, DIM);
    }
    rhythm::one(line, columns, "  ⎿ ", DIM, 4)
}

/// Facts retained from the former singleton while a new chapter takes shape.
/// The caller uses only as many rows as its content height requires.
pub fn receipts(tool: &Tool<'_>, columns: usize) -> Vec<Line> {
    if tool.name == "collaboration" {
        return agents::receipts(tool, columns);
    }
    let Some(result) = tool.result else {
        return activity(std::slice::from_ref(tool), columns)
            .into_iter()
            .collect();
    };
    let mut rows = Vec::new();
    if matches!(tool.name, "bash" | "exec" | "process") {
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
            Status::Unknown => None,
        };
        if let Some(outcome) = outcome {
            rows.push(rhythm::one(
                Line::new(&outcome, DIM),
                columns,
                "  ⎿ ",
                DIM,
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
                Line::new(&format!("{output_lines} output lines captured"), DIM),
                columns,
                "  ⎿ ",
                DIM,
                4,
            ));
        }
        let command = rhythm::payload(tool);
        if !rhythm::description(tool).is_empty() && !command.is_empty() {
            rows.push(rhythm::one(
                Line::new(&format!("Command: {}", text::clean(&command)), DIM),
                columns,
                "  ⎿ ",
                DIM,
                5,
            ));
        }
    } else {
        rows.push(rhythm::one(
            summary::line(tool, Status::of(tool), None),
            columns,
            "  ⎿ ",
            DIM,
            3,
        ));
        rows.extend(content::notices(tool.name, result, columns, false));
    }
    rows
}

/// One primary row for commands and exceptional calls; errors add detail below.
pub fn exception(tool: &Tool<'_>, columns: usize) -> Vec<Line> {
    if tool.name == "collaboration" {
        return agents::member(tool, columns);
    }
    let state = Status::of(tool);
    if matches!(
        tool.name,
        "read" | "list" | "search" | "map" | "web" | "browser"
    ) && state == Status::Running
    {
        let (_, target) = heading(tool);
        let mut line = named(tool);
        if !target.is_empty() {
            line.push(&format!(" {}", text::clean(&target)), BODY);
        }
        let suffix = format!(
            "  Running · {}",
            timer::duration(tool.presentation.duration().unwrap_or_default())
        );
        line = line.shortened(columns.saturating_sub(4 + text::cells(&suffix)));
        line.push(&suffix, DIM);
        return vec![rhythm::one(line, columns, "  ⎿ ", DIM, 2)];
    }
    if matches!(tool.name, "list" | "search" | "map")
        && let Some(result) = tool.result
        && (result.get("complete") == Some(&crate::json::Value::Bool(false))
            || result
                .get("errors_count")
                .and_then(crate::json::Value::as_usize)
                .unwrap_or(0)
                > 0
            || result
                .get("errors")
                .and_then(crate::json::Value::as_array)
                .is_some_and(|errors| !errors.is_empty()))
    {
        let mut line = named(tool);
        line.push(
            &format!(" {}", summary::line(tool, state, None).plain()),
            BODY,
        );
        return vec![rhythm::one(line, columns, "    ", DIM, 2)];
    }
    if matches!(tool.name, "bash" | "exec" | "process") {
        let mut line = named(tool);
        line.push(" ", BODY);
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
            Status::Unknown => Some("unknown".into()),
        };
        if let Some(detail) = detail {
            let suffix = format!("  {detail}");
            line = line.shortened(columns.saturating_sub(4 + text::cells(&suffix)));
            line.push(&suffix, DIM);
        }
        return vec![rhythm::one(line, columns, "  ⎿ ", DIM, 2)];
    }
    if matches!(
        tool.name,
        "read" | "list" | "search" | "map" | "web" | "browser"
    ) && matches!(state, Status::Running | Status::Done)
    {
        return Vec::new();
    }
    let mut title = named(tool);
    if tool.name == "browser" {
        title.push(&format!(" {}", field(tool.arguments, "action")), MUTED);
    }
    title.push(" ", BODY);
    let description = rhythm::description(tool);
    if !description.is_empty() {
        title.push(&text::clean(description), BODY);
    } else if matches!(tool.name, "bash" | "exec" | "process" | "browser") {
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
        Status::Failed | Status::Unknown | Status::Done => None,
    };
    if let Some(detail) = detail {
        let suffix = format!("  {detail}");
        title = title.shortened(columns.saturating_sub(4 + text::cells(&suffix)));
        title.push(&suffix, DIM);
    }
    let mut rows = vec![rhythm::one(title, columns, "  ⎿ ", DIM, 2)];
    if state == Status::Failed {
        rows.extend(failure::rows(tool, columns, "    "));
    }
    rows
}
