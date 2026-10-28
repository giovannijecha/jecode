use super::{App, editor::Editor, terminal::Key};
use crate::todo::{Plan, Task};

#[derive(Default)]
pub struct Panel {
    pub filter: Editor,
    pub selected: usize,
    pub detail_id: Option<String>,
    pub offset: usize,
}

impl Panel {
    pub fn filtered<'a>(&self, plan: &'a Plan) -> Vec<&'a Task> {
        let query = self.filter.text.trim().to_lowercase();
        plan.tasks
            .iter()
            .filter(|task| {
                query.is_empty()
                    || [
                        task.id.as_str(),
                        task.title.as_str(),
                        task.status.as_str(),
                        task.note.as_str(),
                        task.agent.as_deref().unwrap_or(""),
                    ]
                    .iter()
                    .any(|part| part.to_lowercase().contains(&query))
            })
            .collect()
    }

    pub fn current<'a>(&self, plan: &'a Plan) -> Option<&'a Task> {
        if let Some(id) = &self.detail_id {
            plan.tasks.iter().find(|task| &task.id == id)
        } else {
            self.filtered(plan).get(self.selected).copied()
        }
    }
}

impl App {
    pub(super) fn refresh_todo(&mut self) -> bool {
        let messages = self.archive.messages.lock().unwrap();
        if messages.len() == self.todo_message_len {
            return false;
        }
        self.todo_message_len = messages.len();
        let plan = crate::todo::latest(&messages);
        drop(messages);
        if plan == self.state.todo_plan {
            return false;
        }
        self.state.todo_plan = plan;
        if let Some(panel) = &mut self.state.todo_panel {
            if let Some(plan) = &self.state.todo_plan {
                panel.selected = panel
                    .selected
                    .min(panel.filtered(plan).len().saturating_sub(1));
            } else {
                panel.selected = 0;
            }
        }
        self.state.changed();
        true
    }

    pub(super) fn open_todo(&mut self) {
        self.refresh_todo();
        self.close_ask();
        self.state.agents_panel = None;
        self.state.processes = None;
        self.state.selector = None;
        self.state.information = None;
        self.state.suggestions.dismiss();
        self.state.todo_panel = Some(Panel::default());
        self.state.changed();
    }

    pub(super) fn todo_text(&mut self, text: &str) {
        if let Some(panel) = &mut self.state.todo_panel
            && panel.detail_id.is_none()
            && panel.filter.insert(text)
        {
            panel.selected = 0;
        }
    }

    pub(super) fn todo_scroll(&mut self, amount: i16) {
        for _ in 0..amount.unsigned_abs().min(30) {
            self.todo_key(Key {
                code: if amount < 0 { 38 } else { 40 },
                modifiers: 0,
                character: 0,
            });
        }
    }

    pub(super) fn todo_key(&mut self, key: Key) {
        let room = super::view::todo_capacity(&self.state).max(1);
        let count = super::view::todo_detail_rows(&self.state);
        let Some(panel) = &mut self.state.todo_panel else {
            return;
        };
        if key.code == 27 || key.ctrl() && key.code == 67 {
            if panel.detail_id.is_some() {
                panel.detail_id = None;
                panel.offset = 0;
            } else {
                self.state.todo_panel = None;
            }
            return;
        }
        if key.ctrl() || key.alt() {
            return;
        }
        if panel.detail_id.is_some() {
            let end = count.saturating_sub(room);
            panel.offset = match key.code {
                38 => panel.offset.saturating_sub(1),
                40 => (panel.offset + 1).min(end),
                33 => panel.offset.saturating_sub(room),
                34 => (panel.offset + room).min(end),
                36 => 0,
                35 => end,
                _ => panel.offset,
            };
            return;
        }
        let Some(plan) = &self.state.todo_plan else {
            return;
        };
        if key.code == 13 {
            panel.detail_id = panel.current(plan).map(|task| task.id.clone());
            panel.offset = 0;
            return;
        }
        if matches!(key.code, 8 | 46) {
            if panel.filter.key(key) {
                panel.selected = 0;
            }
            return;
        }
        let last = panel.filtered(plan).len().saturating_sub(1);
        panel.selected = match key.code {
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
