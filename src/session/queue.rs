//! New user guidance is inserted only between complete model/tool steps.
use super::{Event, Failure, history::History, worker::Context};
use std::{collections::VecDeque, sync::Mutex};

/// Pending guidance has one owner. Claiming the oldest entry and withdrawing the
/// newest entry are mutually exclusive decisions under this lock.
#[derive(Default)]
pub(crate) struct Pending(Mutex<VecDeque<String>>);
impl Pending {
    pub(crate) fn push(&self, text: &str) -> bool {
        let mut queue = self.0.lock().unwrap_or_else(|error| error.into_inner());
        if queue.len() >= 8 {
            return false;
        }
        queue.push_back(text.to_owned());
        true
    }
    pub(crate) fn claim(&self) -> Option<String> {
        self.0
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .pop_front()
    }
    pub(crate) fn withdraw_latest(&self) -> Option<String> {
        self.0
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .pop_back()
    }
    #[cfg(test)]
    pub(crate) fn snapshot(&self) -> Vec<String> {
        self.0
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .iter()
            .cloned()
            .collect()
    }
}
pub(super) struct Guidance {
    pub after_step: usize,
    pub text: String,
}
#[cfg(test)]
#[path = "queue_tests.rs"]
mod tests;
pub(super) fn take(history: &mut History, context: &Context) -> Result<bool, Failure> {
    let mut received = Vec::new();
    while let Some(text) = context.guidance.claim() {
        let turn = history.turns.last_mut().ok_or(Failure::Worker)?;
        turn.guidance.push(Guidance {
            after_step: turn.steps.len(),
            text: text.clone(),
        });
        received.push(text);
    }
    if received.is_empty() {
        return Ok(false);
    }
    if let Err(error) = history.checkpoint() {
        if let Some(turn) = history.turns.last_mut() {
            turn.guidance.truncate(turn.guidance.len() - received.len());
        }
        for text in received {
            let _ = context.send(Event::GuidanceReturned(text), false);
        }
        return Err(error);
    }
    for text in received {
        if context
            .send(
                Event::Guidance {
                    text,
                    new_turn: false,
                },
                false,
            )
            .is_break()
        {
            return Err(Failure::Cancelled);
        }
    }
    Ok(true)
}
pub(super) fn return_pending(context: &Context) {
    while let Some(text) = context.guidance.claim() {
        let _ = context.send(Event::GuidanceReturned(text), false);
    }
}
