//! New user guidance is inserted only between complete model/tool steps.
use super::{Event, Failure, history::History, worker::Context};
pub(super) struct Guidance {
    pub after_step: usize,
    pub text: String,
}
#[cfg(test)]
#[path = "queue_tests.rs"]
mod tests;
pub(super) fn take(history: &mut History, context: &Context) -> Result<bool, Failure> {
    let mut received = Vec::new();
    while let Ok(text) = context.guidance.try_recv() {
        let turn = history.turns.last_mut().ok_or(Failure::Worker)?;
        if turn.guidance.len() >= 64 {
            let _ = context.send(Event::GuidanceReturned(text), false);
            continue;
        }
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
    while let Ok(text) = context.guidance.try_recv() {
        let _ = context.send(Event::GuidanceReturned(text), false);
    }
}
