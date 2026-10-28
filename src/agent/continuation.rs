use super::Agent;
use crate::{
    events::{Event, EventSink},
    sessions::Stage,
};

impl Agent {
    /// Restored, unfinished orchestration belongs to the main agent as well.
    /// A historical receipt alone does not reopen an assignment already closed.
    pub fn has_open_collaboration_report(&self) -> bool {
        self.collaboration.snapshot().iter().any(|record| {
            matches!(
                record.get("state").and_then(crate::json::Value::as_str),
                Some("returned" | "failed")
            ) && record
                .get("report")
                .and_then(|report| report.get("version"))
                == record.get("sent_version")
                && record.get("report").is_some_and(|report| {
                    report
                        .get("version")
                        .and_then(crate::json::Value::as_usize)
                        .is_some()
                })
        })
    }

    /// Continue the main conversation after a live collaborator return.
    /// No synthetic user request is added to the transcript.
    pub fn continue_collaboration(&mut self, events: &mut impl EventSink) -> Result<bool, String> {
        if self.child
            || self.has_pending_question()
            || self.cancellation.requested()
            || self.collaboration.latest_return_sequence() <= self.collaboration_seen
        {
            return Ok(false);
        }
        if self.prepared.is_some() {
            return Err("A prompt is already prepared".into());
        }
        self.refresh_project_instructions()?;
        self.refresh_skills();
        self.checkpoint(Stage::Continue)?;
        let observer = self.collaboration();
        let mut cursor = observer.events_since(0).0;
        let mut partial = String::new();
        let mut result = self.drive_turn(&mut |event| {
            let (next, receipts) = observer.events_since(cursor);
            cursor = next;
            for receipt in receipts {
                events.emit(Event::Collaboration { event: receipt })?;
            }
            match &event {
                Event::Streaming { text } => partial = text.clone(),
                Event::Message { .. }
                | Event::Waiting { .. }
                | Event::Recovering { .. }
                | Event::RequestDiscarded => partial.clear(),
                _ => {}
            }
            events.emit(event)
        });
        let (_, receipts) = observer.events_since(cursor);
        for receipt in receipts {
            if let Err(error) = events.emit(Event::Collaboration { event: receipt }) {
                if result.is_ok() {
                    result = Err(error);
                }
                break;
            }
        }
        if self.questions.is_suspended() {
            return self.checkpoint(Stage::Preserve).map(|()| true);
        }
        if let Err(error) = &result {
            self.record_turn_error(error, &partial);
        }
        let saved = self.checkpoint(Stage::Ready);
        if result.is_ok()
            && let Err(error) = &saved
        {
            self.record_turn_error(error, "");
            let _ = self.checkpoint(Stage::Ready);
        }
        result.and(saved).map(|()| true)
    }
}
