//! What a Work chapter did: the outcome counts its title and badge carry, and
//! the facts row below it.

use super::{Status, Tool, field};
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

pub(super) fn facts(tools: &[Tool<'_>], outcomes: &Outcomes) -> Line {
    let mut files = BTreeSet::new();
    let mut other_reads = 0;
    let mut listings = 0;
    let mut searches = 0;
    let mut maps = 0;
    let mut web = 0;
    let mut screenshots = 0;
    let mut snapshots = 0;
    let mut browser = 0;
    let mut commands = 0;
    let mut processes = 0;
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
            "bash" | "exec" => commands += 1,
            "list" => listings += 1,
            "search" => searches += 1,
            "map" => maps += 1,
            "process" => processes += 1,
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
        (maps, "map"),
        (web, "web call"),
        (screenshots, "screenshot"),
        (snapshots, "snapshot"),
        (browser, "browser action"),
        (commands, "command"),
        (processes, "process call"),
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
    let mut line = Line::new(&parts.join(" · "), DIM);
    for (count, label, style) in [
        (outcomes.stopped, "stopped", DIM),
        (outcomes.running, "running", DIM),
        (outcomes.background, "in background", DIM),
    ] {
        if count > 0 {
            if !line.spans.is_empty() {
                line.push(" · ", DIM);
            }
            line.push(&format!("{count} {label}"), style);
        }
    }
    if line.spans.is_empty() {
        Line::new(&format!("{} completed", outcomes.done), DIM)
    } else {
        line
    }
}
