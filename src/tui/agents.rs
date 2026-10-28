//! Read-only inspection state for collaborators in the current conversation.

use super::{App, editor::Editor, terminal::Key};
use crate::json::Value;

pub(super) struct Panel {
    pub filter: Editor,
    pub selected: usize,
    pub detail: bool,
    pub detail_id: Option<String>,
    pub tab: usize,
    pub offset: usize,
    pub follow: bool,
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

pub(super) fn status(record: &Value) -> &'static str {
    match value(record, "state") {
        "running" => "Running",
        "returned" | "failed" if record.get("reviewed") == Some(&Value::Bool(true)) => "Reviewed",
        "returned"
            if list(record, "checks")
                .iter()
                .filter_map(|check| check.as_str())
                .any(|check| check.to_ascii_lowercase().contains("fail")) =>
        {
            "Failed check · pending review by main agent"
        }
        "returned" => "Pending review by main agent",
        "failed" => "Failed · pending review by main agent",
        "stopping" => "Stop requested",
        "stopped" => "Stopped",
        "unknown" => "Outcome unknown",
        _ => "Queued",
    }
}

impl Panel {
    fn reconcile(&mut self, records: &[Value]) {
        if self.detail && self.current(records).is_none() {
            self.detail = false;
            self.detail_id = None;
            self.tab = 0;
            self.offset = 0;
            self.follow = true;
        }
        self.selected = self
            .selected
            .min(self.filtered(records).len().saturating_sub(1));
    }

    pub fn filtered(&self, records: &[Value]) -> Vec<usize> {
        let query = self.filter.text.trim().to_lowercase();
        records
            .iter()
            .enumerate()
            .filter(|(_, record)| {
                query.is_empty()
                    || format!(
                        "{} {} {} {} {}",
                        label(record),
                        value(record, "title"),
                        value(record, "state"),
                        status(record),
                        value(record, "activity")
                    )
                    .to_lowercase()
                    .contains(&query)
            })
            .map(|(index, _)| index)
            .collect()
    }

    pub fn current<'a>(&self, records: &'a [Value]) -> Option<&'a Value> {
        if self.detail {
            return records
                .iter()
                .find(|record| Some(value(record, "id")) == self.detail_id.as_deref());
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
            detail: false,
            detail_id: None,
            tab: 0,
            offset: 0,
            follow: true,
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
            let room = super::view::agents_capacity(&self.state).max(1);
            let end = super::view::agents_detail_rows(&self.state).saturating_sub(room);
            if let Some(panel) = &mut self.state.agents_panel {
                panel.reconcile(&self.state.collaboration);
                if panel.detail && panel.tab == 0 && panel.follow {
                    panel.offset = end;
                }
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
        dirty
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
        if panel.detail {
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
        let total = super::view::agents_detail_rows(&self.state);
        let Some(panel) = &mut self.state.agents_panel else {
            return;
        };
        if panel.detail {
            let end = total.saturating_sub(room);
            let offset = if panel.tab == 0 && panel.follow {
                end
            } else {
                panel.offset
            };
            panel.offset = match code {
                38 => offset.saturating_sub(1),
                40 => (offset + 1).min(end),
                33 => offset.saturating_sub(room),
                34 => (offset + room).min(end),
                36 => 0,
                35 => end,
                _ => offset,
            };
            if panel.tab == 0 && matches!(code, 33..=40) {
                panel.follow = code == 35;
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
                if panel.detail {
                    panel.detail = false;
                    panel.detail_id = None;
                    panel.offset = 0;
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
        if panel.detail {
            if matches!(key.code, 37 | 39 | 9) {
                panel.tab = if key.code == 37 {
                    (panel.tab + 2) % 3
                } else {
                    (panel.tab + 1) % 3
                };
                panel.offset = 0;
                panel.follow = true;
                return;
            }
        } else if key.code == 13 && panel.current(&self.state.collaboration).is_some() {
            panel.detail_id = panel
                .current(&self.state.collaboration)
                .map(|record| value(record, "id").to_string());
            panel.detail = true;
            panel.tab = 0;
            panel.offset = 0;
            panel.follow = true;
            return;
        } else if matches!(key.code, 8 | 46) {
            if panel.filter.key(key) {
                panel.selected = 0;
            }
            return;
        }
        self.agents_move(key.code);
    }
}

#[cfg(test)]
#[path = "agents/tests.rs"]
mod tests;
