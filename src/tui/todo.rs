//! `/todo` reads the model's plan; the conversation shows only the call that
//! creates one.

use super::{App, terminal::Key};
use crate::{
    json::Value,
    todo::{self, Phase, Plan},
};

/// The open menu. Nothing is selected: the window follows the plan until
/// the reader scrolls it.
#[derive(Default)]
pub struct Panel {
    /// The first visible task once scrolled; `None` follows the plan.
    pub offset: Option<usize>,
}

impl Panel {
    /// The first of `room` visible rows. It opens one task above the first
    /// unfinished one.
    pub fn start(&self, plan: &Plan, records: &[Value], room: usize) -> usize {
        let end = plan.tasks.len().saturating_sub(room);
        self.offset
            .unwrap_or_else(|| {
                plan.tasks
                    .iter()
                    .position(|task| !Phase::of(task, todo::owner(task, records)).finished())
                    .map_or(0, |first| first.saturating_sub(1))
            })
            .min(end)
    }
}

/// The plan as the transcript's own todo_list results left it, in call order.
/// It decides which calls create a plan and so have a conversation row.
#[derive(Default)]
pub struct Calls {
    plan: Option<Plan>,
}

impl Calls {
    pub fn creates(&self, arguments: &Value) -> bool {
        todo::creates(self.plan.as_ref(), arguments)
    }

    pub fn finished(&mut self, result: &Value) {
        if result.get("error").is_none()
            && let Ok(plan) = Plan::parse(result)
            && plan.revision > 0
        {
            self.plan = Some(plan);
        }
    }
}

impl App {
    /// Reads the latest plan when the history grew. A checkpoint copying the
    /// history holds it briefly; the next loop pass reads it then.
    pub(super) fn refresh_todo(&mut self) -> bool {
        let messages = match self.archive.messages.try_lock() {
            Ok(messages) => messages,
            Err(std::sync::TryLockError::WouldBlock) => return false,
            Err(std::sync::TryLockError::Poisoned(error)) => panic!("{error}"),
        };
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

    pub(super) fn todo_scroll(&mut self, amount: i16) {
        for _ in 0..amount.unsigned_abs().min(30) {
            self.todo_key(Key {
                code: if amount < 0 { 38 } else { 40 },
                modifiers: 0,
                character: 0,
            });
        }
    }

    /// ↑↓, PgUp/PgDn and Home/End scroll; Esc closes.
    pub(super) fn todo_key(&mut self, key: Key) {
        if key.code == 27 || key.ctrl() && key.code == 67 {
            self.state.todo_panel = None;
            return;
        }
        if key.ctrl() || key.alt() {
            return;
        }
        let room = super::view::todo_capacity(&self.state).max(1);
        let (Some(panel), Some(plan)) = (&mut self.state.todo_panel, &self.state.todo_plan) else {
            return;
        };
        let start = panel.start(plan, &self.state.collaboration, room);
        let end = plan.tasks.len().saturating_sub(room);
        panel.offset = Some(match key.code {
            38 => start.saturating_sub(1),
            40 => (start + 1).min(end),
            33 => start.saturating_sub(room),
            34 => (start + room).min(end),
            36 => 0,
            35 => end,
            _ => return,
        });
    }
}
