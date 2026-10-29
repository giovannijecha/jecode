//! `/agents`: the collaborators of this conversation, and inside one, its own
//! conversation, read only.

use super::{App, editor::Editor, terminal::Key};
use crate::json::Value;

pub(super) mod conversation;

pub(super) struct Panel {
    pub filter: Editor,
    pub selected: usize,
    /// The collaborator whose conversation replaces the main one.
    pub inside: Option<String>,
    /// First conversation row shown inside, unless following the end.
    pub offset: usize,
    pub follow: bool,
    /// The open collaborator's file changes with their hunks.
    pub changes: conversation::Changes,
    /// The open collaborator's rendered conversation.
    pub rows: super::view::agents::Cache,
}

pub(super) fn value<'a>(record: &'a Value, key: &str) -> &'a str {
    record.get(key).and_then(Value::as_str).unwrap_or("")
}

pub(super) fn number(record: &Value, key: &str) -> usize {
    record.get(key).and_then(Value::as_usize).unwrap_or(0)
}

/// These actions have durable collaboration receipts in the transcript.
/// Inspection and workspace actions keep their ordinary tool cards.
pub(super) fn receipt_replaces_tool(arguments: &Value) -> bool {
    matches!(
        value(arguments, "action"),
        "create" | "update" | "review" | "stop" | "close"
    )
}

pub(super) fn list<'a>(record: &'a Value, key: &str) -> Vec<&'a Value> {
    record
        .get(key)
        .and_then(Value::as_array)
        .map_or_else(Vec::new, |items| items.iter().collect())
}

pub(super) fn label(record: &Value) -> String {
    format!("AGENT {}", number(record, "number"))
}

/// The first failed command check. Current records carry structured tool
/// results; older snapshots may only have the command summaries in `checks`.
pub(super) fn failed_check(record: &Value) -> Option<&str> {
    let mut structured = false;
    for tool in list(record, "tools") {
        if !matches!(value(tool, "name"), "bash" | "exec") {
            continue;
        }
        let Some(result) = tool.get("result") else {
            continue;
        };
        structured = true;
        if failed_command(result) {
            return Some(value(tool, "text"))
                .filter(|summary| !summary.is_empty())
                .or_else(|| result.get("error").and_then(Value::as_str))
                .or(Some("Command failed"));
        }
    }
    if structured {
        return None;
    }
    list(record, "checks")
        .into_iter()
        .filter_map(Value::as_str)
        .find(|check| {
            let summary = check
                .split_once(": ")
                .map_or(*check, |(_, summary)| summary);
            summary
                .strip_prefix("exit ")
                .and_then(|code| {
                    code.split(|ch: char| ch.is_whitespace() || ch == ';')
                        .next()
                })
                .and_then(|code| code.parse::<i32>().ok())
                .is_some_and(|code| code != 0)
                || summary.contains("timed out")
                || summary.contains("cancelled")
                || summary.to_ascii_lowercase().contains("fail")
        })
        .map(|check| check.split_once(": ").map_or(check, |(_, summary)| summary))
}

fn failed_command(result: &Value) -> bool {
    result.get("error").is_some()
        || result.get("cancelled") == Some(&Value::Bool(true))
        || result.get("timed_out") == Some(&Value::Bool(true))
        || matches!(
            crate::events::process_status(result),
            Some("stopped" | "timed_out")
        )
        || result
            .get("exit_code")
            .is_some_and(|code| code != &Value::number(0) && code != &Value::Null)
}

/// A collaborator still on its assignment, which quitting would interrupt.
pub(super) fn working(record: &Value) -> bool {
    matches!(status(record), "running" | "stopping" | "queued")
}

pub(super) fn status(record: &Value) -> &'static str {
    match value(record, "state") {
        "running" => "running",
        "returned" | "failed" if record.get("reviewed") == Some(&Value::Bool(true)) => "reviewed",
        "returned" if failed_check(record).is_some() => "failed check · main review",
        "returned" => "returned · main review",
        "failed" => "failed · main review",
        "stopping" => "stopping",
        "stopped" => "stopped",
        "unknown" => "outcome unknown",
        _ => "queued",
    }
}

impl Panel {
    fn reconcile(&mut self, records: &[Value]) {
        if self.inside.is_some() && self.current(records).is_none() {
            self.leave();
        }
        self.selected = self
            .selected
            .min(self.filtered(records).len().saturating_sub(1));
    }

    fn leave(&mut self) {
        self.inside = None;
        self.offset = 0;
        self.follow = true;
        self.rows.clear();
    }

    pub fn filtered(&self, records: &[Value]) -> Vec<usize> {
        let query = self.filter.text.trim().to_lowercase();
        records
            .iter()
            .enumerate()
            .filter(|(_, record)| {
                query.is_empty()
                    || format!(
                        "{} {} {} {}",
                        label(record),
                        value(record, "title"),
                        status(record),
                        value(record, "activity")
                    )
                    .to_lowercase()
                    .contains(&query)
            })
            .map(|(index, _)| index)
            .collect()
    }

    /// The collaborator shown inside, or the one selected in the list.
    pub fn current<'a>(&self, records: &'a [Value]) -> Option<&'a Value> {
        if let Some(id) = &self.inside {
            return records.iter().find(|record| value(record, "id") == id);
        }
        let filtered = self.filtered(records);
        records.get(*filtered.get(self.selected)?)
    }
}

impl Default for Panel {
    fn default() -> Self {
        Self {
            filter: Editor::default(),
            selected: 0,
            inside: None,
            offset: 0,
            follow: true,
            changes: conversation::Changes::default(),
            rows: super::view::agents::Cache::default(),
        }
    }
}

impl App {
    pub(super) fn start_collaboration_continuation(&mut self) -> bool {
        if !self.collaboration_auto_resume
            || !self.collaboration_return_pending
            || !self.idle()
            || !self.state.queue.is_empty()
            || self.state.queue.is_editing()
            || self.state.selector.is_some()
            || self.state.quitting.is_some()
            || self.state.question.is_some()
        {
            return false;
        }
        let Some(agent) = self.agent.take() else {
            return false;
        };
        self.collaboration_return_pending = false;
        self.state.begin_tool_turn();
        self.state.activity = Some(super::activity::Activity::new());
        self.state.backend_failure = None;
        self.state.status = "Reviewing collaborator work…".into();
        self.worker = Some(super::Worker::continue_collaboration(agent));
        true
    }

    pub(super) fn refresh_agents(&mut self) -> bool {
        let records: Vec<Value> = self
            .collaboration_observer
            .snapshot()
            .into_iter()
            .map(|record| self.archive.redactor.value(&record))
            .collect();
        let mut dirty = self.state.collaboration != records;
        if dirty {
            self.state.collaboration = records;
            let now = std::time::Instant::now();
            let clocks = &mut self.state.agent_clocks;
            clocks.retain(|id, _| {
                self.state
                    .collaboration
                    .iter()
                    .any(|record| value(record, "id") == id && value(record, "state") == "running")
            });
            for record in &self.state.collaboration {
                if value(record, "state") == "running" {
                    clocks.entry(value(record, "id").into()).or_insert(now);
                }
            }
            if let Some(panel) = &mut self.state.agents_panel {
                panel.reconcile(&self.state.collaboration);
            }
            self.state.changed();
        }
        let (cursor, events) = self
            .collaboration_observer
            .events_since(self.collaboration_cursor);
        self.collaboration_cursor = cursor;
        for event in events {
            dirty |= self.show_collaboration(event);
        }
        dirty | self.refresh_agent_changes()
    }

    /// Reads the open collaborator's changes again when it changed more
    /// files. Its session holds the hunks; its record keeps only counts.
    fn refresh_agent_changes(&mut self) -> bool {
        let Some(panel) = &mut self.state.agents_panel else {
            return false;
        };
        let (Some(id), Some(record)) = (&panel.inside, panel.current(&self.state.collaboration))
        else {
            return false;
        };
        let counted = conversation::changed_calls(record);
        if panel.changes.id == *id && panel.changes.counted == counted {
            return false;
        }
        let calls = if counted == 0 {
            std::collections::HashMap::new()
        } else {
            self.collaboration_observer
                .changes(id)
                .into_iter()
                .map(|(call, change)| (call, change.redacted(&self.archive.redactor)))
                .collect()
        };
        panel.changes = conversation::Changes {
            id: id.clone(),
            counted,
            calls,
        };
        panel.rows.clear();
        self.state.changed();
        true
    }

    pub(super) fn show_collaboration(&mut self, event: Value) -> bool {
        let sequence = number(&event, "sequence");
        if !self.collaboration_seen.insert(sequence) {
            return false;
        }
        if value(&event, "kind") == "returned" {
            self.collaboration_return_pending = true;
        }
        self.state
            .collaboration_event(self.archive.redactor.value(&event));
        true
    }

    pub(super) fn reset_agents_observer(&mut self) {
        if let Some(agent) = &self.agent {
            self.collaboration_observer = agent.collaboration();
        }
        self.collaboration_cursor = 0;
        self.collaboration_seen.clear();
        self.collaboration_receipt_calls.clear();
        self.collaboration_return_pending = false;
        self.collaboration_auto_resume = true;
        self.refresh_agents();
    }
    pub(super) fn open_agents(&mut self) {
        self.close_ask();
        self.state.information = None;
        self.state.selector = None;
        self.state.processes = None;
        self.state.suggestions.dismiss();
        self.state.agents_panel = Some(Panel::default());
        self.state.todo_panel = None;
        self.refresh_agents();
    }

    pub(super) fn agents_text(&mut self, text: &str) {
        let Some(panel) = &mut self.state.agents_panel else {
            return;
        };
        if panel.inside.is_some() {
            return;
        }
        if panel.filter.insert(text) {
            panel.selected = 0;
        }
    }

    pub(super) fn agents_scroll(&mut self, amount: i16) {
        for _ in 0..amount.unsigned_abs().min(30) {
            self.agents_move(if amount < 0 { 38 } else { 40 });
        }
    }

    fn agents_move(&mut self, code: u16) {
        let room = super::view::agents_capacity(&self.state).max(1);
        let (total, inside_room) = super::view::agents_inside_rows(&self.state);
        let Some(panel) = &mut self.state.agents_panel else {
            return;
        };
        if panel.inside.is_some() {
            let room = inside_room.max(1);
            let end = total.saturating_sub(room);
            let offset = if panel.follow {
                end
            } else {
                panel.offset.min(end)
            };
            panel.offset = match code {
                38 => offset.saturating_sub(1),
                40 => (offset + 1).min(end),
                33 => offset.saturating_sub(room),
                34 => (offset + room).min(end),
                36 => 0,
                _ => offset,
            };
            if matches!(code, 33..=40) {
                // End follows; reaching the end by scrolling also does.
                panel.follow = code == 35 || matches!(code, 34 | 40) && panel.offset == end;
            }
        } else {
            let last = panel
                .filtered(&self.state.collaboration)
                .len()
                .saturating_sub(1);
            panel.selected = match code {
                38 => panel.selected.saturating_sub(1),
                40 => (panel.selected + 1).min(last),
                33 => panel.selected.saturating_sub(room),
                34 => (panel.selected + room).min(last),
                36 => 0,
                35 => last,
                _ => panel.selected,
            };
        }
    }

    pub(super) fn agents_key(&mut self, key: Key) {
        if key.code == 27 || key.ctrl() && key.code == 67 {
            if let Some(panel) = &mut self.state.agents_panel {
                if panel.inside.is_some() {
                    panel.leave();
                } else {
                    self.state.agents_panel = None;
                }
            }
            return;
        }
        if key.ctrl() || key.alt() {
            return;
        }
        let Some(panel) = &mut self.state.agents_panel else {
            return;
        };
        if panel.inside.is_none() {
            if key.code == 13 {
                if let Some(record) = panel.current(&self.state.collaboration) {
                    panel.inside = Some(value(record, "id").to_string());
                    panel.offset = 0;
                    panel.follow = true;
                }
                self.refresh_agent_changes();
                return;
            }
            if matches!(key.code, 8 | 46) {
                if panel.filter.key(key) {
                    panel.selected = 0;
                }
                return;
            }
        }
        self.agents_move(key.code);
    }
}

#[cfg(test)]
#[path = "agents/tests.rs"]
mod tests;
