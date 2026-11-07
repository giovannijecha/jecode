//! Process completion delivery and the continuation available in this frontend.

use super::Agent;
use crate::events::{Event, EventSink};
use crate::json::Value;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Resume {
    Automatic,
    OnInput,
    None,
}

impl Agent {
    /// This agent answers one prompt and the program exits.
    pub fn set_single_run(&mut self) {
        self.process_resume = Resume::None;
    }

    /// A blocking line reader can continue only after the next entered line.
    pub fn set_process_resume_on_input(&mut self) {
        self.process_resume = Resume::OnInput;
    }

    pub(super) fn background_resume(&self) -> &'static str {
        match (self.child, self.process_resume) {
            (true, _) | (_, Resume::None) => "none",
            (_, Resume::OnInput) => "on_input",
            (_, Resume::Automatic) => "automatic",
        }
    }

    #[cfg(test)]
    pub fn set_process_wait(&self, wait: std::time::Duration) {
        self.tools.set_default_wait(wait);
    }
    /// Whether a process that ended by itself still needs a main-agent
    /// response. A stopped turn or a pending question keeps it waiting for
    /// the user; the next request carries it either way.
    pub fn can_continue_processes(&self) -> bool {
        !self.ends_with_answer()
            && self.prepared.is_none()
            && !self.has_pending_question()
            && !self.cancellation.requested()
            && self.tools.process_completion_waiting()
    }

    /// Continue the conversation after a process ended. The completion
    /// arrives as session state, so no user request joins the transcript.
    pub fn continue_processes(&mut self, events: &mut impl EventSink) -> Result<bool, String> {
        if !self.can_continue_processes() {
            return Ok(false);
        }
        self.continue_turn(events)
    }

    /// A collaborator's answer returns its assignment, and a single prompt's
    /// answer ends the run: nothing would resume either later.
    pub(super) fn ends_with_answer(&self) -> bool {
        self.child || self.process_resume == Resume::None
    }

    /// Marks finished processes delivered to the model and tells the sink.
    /// The UI may already have saved the transcript event before this point.
    pub(super) fn deliver_completions(
        &self,
        completions: Vec<crate::tools::processes::Completion>,
        emit: &mut impl FnMut(Event) -> Result<(), String>,
    ) -> Result<(), String> {
        if completions.is_empty() {
            return Ok(());
        }
        let ids: Vec<_> = completions.iter().map(|done| done.id.clone()).collect();
        self.tools.processes_reported(&ids);
        let after_message = self.messages.lock().unwrap().len();
        for process in completions {
            let mut event = process.value();
            if let Value::Object(fields) = &mut event {
                fields.insert("type".into(), Value::string("process_finished"));
                fields.insert("after_message".into(), Value::number(after_message));
            }
            let mut events = self.events.lock().unwrap();
            if !events.iter().any(|saved| {
                saved.get("type").and_then(Value::as_str) == Some("process_finished")
                    && saved.get("process_id").and_then(Value::as_str) == Some(process.id.as_str())
            }) {
                events.push(event);
            }
            drop(events);
            emit(Event::ProcessFinished { process })?;
        }
        Ok(())
    }

    /// Observes this conversation's persistent processes while a worker owns the agent.
    pub fn processes(&self) -> crate::tools::processes::Observer {
        self.tools.observe_processes(self.redactor.clone())
    }
}
