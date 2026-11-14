//! What a Work chapter did: the outcome counts its title and badge carry, and
//! the facts row below it.

use super::super::Status;
use super::{Tool, field};
use crate::tui::{line::Line, text, theme::DIM};
use std::collections::BTreeSet;

#[derive(Default)]
pub(super) struct Outcomes {
    pub(super) failed: usize,
    running: usize,
    background: usize,
    stopped: usize,
    unknown: usize,
    done: usize,
}

impl Outcomes {
    pub(super) fn add(&mut self, status: Status) {
        match status {
            Status::Failed => self.failed += 1,
            Status::Running => self.running += 1,
            Status::Background => self.background += 1,
            Status::Stopped => self.stopped += 1,
            Status::Unknown => self.unknown += 1,
            Status::Done => self.done += 1,
        }
    }

    pub(super) fn primary(&self) -> Status {
        if self.failed > 0 {
            Status::Failed
        } else if self.running > 0 {
            Status::Running
        } else if self.background > 0 {
            Status::Background
        } else if self.stopped > 0 {
            Status::Stopped
        } else if self.unknown > 0 {
            Status::Unknown
        } else {
            Status::Done
        }
    }
}

pub(super) fn facts(tools: &[Tool<'_>], outcomes: &Outcomes) -> Vec<String> {
    let mut files = BTreeSet::new();
    let mut other_reads = 0;
    let mut listings = 0;
    let mut searches = 0;
    let mut web = 0;
    let mut screenshots = 0;
    let mut snapshots = 0;
    let mut browser = 0;
    let mut commands = 0;
    let mut processes = 0;
    let mut mcp_calls = 0;
    let mut lsp_calls = 0;
    let mut agent_calls = 0;
    let mut agent_events = 0;
    let mut agents_created = 0;
    for tool in tools {
        // Every agent event counts, whatever its outcome; the member row
        // carries a failure or an unknown result.
        if tool.name == "collaboration" && tool.result.is_some() {
            match tool.result.map(|event| field(event, "kind")) {
                Some("created") => {
                    agents_created += tool
                        .result
                        .and_then(|event| event.get("details"))
                        .and_then(crate::json::Value::as_array)
                        .map_or(0, <[_]>::len);
                }
                _ => agent_events += 1,
            }
            continue;
        }
        if Status::of(tool) != Status::Done {
            continue;
        }
        match tool.name {
            "read" if tool.arguments.get("paths").is_some() => {
                for path in tool
                    .arguments
                    .get("paths")
                    .and_then(crate::json::Value::as_array)
                    .unwrap_or(&[])
                    .iter()
                    .filter_map(crate::json::Value::as_str)
                {
                    files.insert(path);
                }
            }
            "read" => {
                let path = field(tool.arguments, "path");
                if ["history:", "output:", "attachment:"]
                    .iter()
                    .any(|prefix| path.starts_with(*prefix))
                    || path.is_empty()
                {
                    other_reads += 1;
                } else if !path.starts_with("skill:") {
                    files.insert(path);
                }
            }
            "bash" | "exec" => commands += 1,
            "list" => listings += 1,
            "search" => searches += 1,
            "process" => processes += 1,
            "mcp" => mcp_calls += 1,
            "lsp" => lsp_calls += 1,
            "web" => web += 1,
            "agent" => agent_calls += 1,
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
        (listings, "listing"),
        (searches, "search"),
        (web, "web call"),
        (screenshots, "screenshot"),
        (snapshots, "snapshot"),
        (browser, "browser action"),
        (commands, "command"),
        (processes, "process call"),
        (mcp_calls, "MCP call"),
        (lsp_calls, "LSP call"),
        (agent_calls, "agent call"),
        (agent_events, "agent event"),
    ] {
        if count > 0 {
            parts.push(text::counted(count, noun));
        }
    }
    if agents_created > 0 {
        parts.push(format!(
            "{} created",
            text::counted(agents_created, "agent")
        ));
    }
    for (count, label) in [
        (outcomes.background, "in background"),
        (outcomes.stopped, "stopped"),
    ] {
        if count > 0 {
            parts.push(format!("{count} {label}"));
        }
    }
    if parts.is_empty() && outcomes.running == 0 {
        parts.push(format!("{} completed", outcomes.done));
    }
    parts
}

/// `N running`, the one fact that disappears while the chapter is live.
pub(super) fn running(outcomes: &Outcomes) -> Option<String> {
    (outcomes.running > 0).then(|| format!("{} running", outcomes.running))
}

/// Facts packed into rows of `width` cells, breaking only between facts; a
/// fact wider than a row wraps on its own. The transient `running` fact joins
/// only a row that has room, so finishing a call never removes a row. At most
/// two rows: when more are folded, the second ends in ` …`.
pub(super) fn rows(parts: &[String], running: Option<String>, width: usize) -> Vec<Line> {
    let fits = |row: &str, part: &str| text::cells(row) + 3 + text::cells(part) <= width;
    let mut packed: Vec<String> = Vec::new();
    for part in parts {
        match packed.last_mut() {
            Some(row) if fits(row, part) => {
                row.push_str(" · ");
                row.push_str(part);
            }
            _ => packed.push(part.clone()),
        }
    }
    if let Some(part) = running {
        let rows = packed.len();
        match packed.last_mut() {
            None => packed.push(part),
            Some(row) if rows <= 2 && fits(row, &part) => {
                row.push_str(" · ");
                row.push_str(&part);
            }
            Some(_) => {}
        }
    }
    let all: Vec<Line> = packed
        .iter()
        .flat_map(|row| Line::new(row, DIM).wrap(width, true))
        .collect();
    let mut rows: Vec<Line> = all.iter().take(2).cloned().collect();
    if all.len() > 2
        && let Some(second) = rows.pop()
    {
        let plain = second.plain();
        let room = width.saturating_sub(2);
        // Drop whole trailing facts first, so the ellipsis never splits one.
        let mut kept = plain.as_str();
        while text::cells(kept) > room {
            match kept.rfind(" · ") {
                Some(at) => kept = &kept[..at],
                None => break,
            }
        }
        let mut second = Line::new(kept, DIM).shortened(room);
        second.push(" …", DIM);
        rows.push(second);
    }
    rows
}
