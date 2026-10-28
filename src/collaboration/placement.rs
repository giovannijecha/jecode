//! What a receipt records about where it happened: the work label of the
//! agent call that wrote it, whether a turn of the main agent was running
//! and whether one of its calls was. The interface places each event from
//! these facts, live and after resume, never from when it sees the event.

use super::{Observer, Shared, State};
use crate::json::Value;
use std::sync::Arc;

/// Marks a turn, or a call, of the main agent as running while it lives.
pub struct Running(Arc<Shared>, fn(&mut State) -> &mut usize);

impl Drop for Running {
    fn drop(&mut self) {
        if let Ok(mut state) = self.0.state.lock() {
            let count = (self.1)(&mut state);
            *count = count.saturating_sub(1);
        }
    }
}

impl Observer {
    /// A handle whose receipts record `work`, the label of the agent call
    /// that uses it, so they join that call's chapter wherever they appear.
    pub(crate) fn calling(&self, work: &str) -> Self {
        Self(Arc::clone(&self.0), Some(work.to_owned()))
    }

    /// Receipts written until the guard drops happened inside a turn.
    pub fn turn(&self) -> Running {
        self.running(|state| &mut state.turns)
    }

    /// Receipts written until the guard drops happened while a call ran,
    /// after its start was shown and before its result was recorded.
    pub fn call(&self) -> Running {
        self.running(|state| &mut state.calls)
    }

    fn running(&self, count: fn(&mut State) -> &mut usize) -> Running {
        *count(&mut self.0.state.lock().unwrap()) += 1;
        Running(Arc::clone(&self.0), count)
    }

    pub(super) fn place(&self, state: &State, event: &mut Value) {
        if let Value::Object(fields) = event {
            fields.insert("in_turn".into(), Value::Bool(state.turns > 0));
            fields.insert("in_call".into(), Value::Bool(state.calls > 0));
            if let Some(work) = &self.1 {
                fields.insert("work".into(), Value::string(work));
            }
        }
    }
}
