use super::Agent;
use crate::{
    attachments::Prompt,
    events::{Event, EventSink},
    sessions::Stage,
};

impl Agent {
    /// Adds submitted messages and reports whether the request context changed.
    pub(super) fn accept_input(&self, events: &mut impl EventSink) -> Result<bool, String> {
        let prompts = self
            .inbox
            .accept(|prompts, queued| self.append_input(prompts, queued))?;
        if prompts.is_empty() {
            return Ok(false);
        }
        events.emit(Event::Accepted { prompts })?;
        self.emit_context(events)?;
        Ok(true)
    }

    /// Records accepted messages and the remaining queue in one checkpoint.
    /// A stopping turn accepts nothing, so its pending messages are paused.
    fn append_input(&self, prompts: &[Prompt], queued: Vec<Prompt>) -> Result<(), String> {
        if self.cancellation.requested() {
            return Err("Operation cancelled".into());
        }
        let before = self.messages.lock().unwrap().len();
        self.messages
            .lock()
            .unwrap()
            .extend(prompts.iter().map(Prompt::message));
        if let Err(error) = self.checkpoint(Stage::Accept(queued)) {
            self.messages.lock().unwrap().truncate(before);
            return Err(error);
        }
        Ok(())
    }
}
