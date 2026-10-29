use super::Agent;
use crate::{
    events::{Event, EventSink},
    json::Value,
    sessions::Stage,
};

/// A continuation sends no new user message, but providers treat a request
/// that ends with an assistant answer as a prefix to extend, or reject it.
/// This request-only note ends the request with a user turn instead; the
/// transcript never stores it.
pub(super) fn end_with_user_turn(messages: &mut Vec<Value>) {
    let last = messages.last();
    let open = match last
        .and_then(|message| message.get("role"))
        .and_then(Value::as_str)
    {
        Some("system") => true,
        Some("assistant") => last
            .and_then(|message| message.get("tool_calls"))
            .and_then(Value::as_array)
            .is_none_or(|calls| calls.is_empty()),
        _ => false,
    };
    if open {
        messages.push(Value::object([
            ("role", Value::string("user")),
            (
                "content",
                Value::string("No new user message. Continue from the current session state."),
            ),
        ]));
    }
}

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
        let _turn = observer.turn();
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

#[cfg(test)]
mod tests {
    use super::*;

    fn message(role: &str, tool_calls: Option<Value>) -> Value {
        let mut message = Value::object([
            ("role", Value::string(role)),
            ("content", Value::string("text")),
        ]);
        if let (Value::Object(fields), Some(calls)) = (&mut message, tool_calls) {
            fields.insert("tool_calls".into(), calls);
        }
        message
    }

    #[test]
    fn only_a_request_ending_with_a_finished_answer_gets_a_user_turn() {
        let call = Value::Array(vec![Value::object([("id", Value::string("c1"))])]);
        for (last, added) in [
            (message("assistant", None), true),
            (message("assistant", Some(Value::Array(vec![]))), true),
            (message("assistant", Some(call)), false),
            (message("user", None), false),
            (message("tool", None), false),
        ] {
            let mut messages = vec![message("system", None), last];
            end_with_user_turn(&mut messages);
            assert_eq!(messages.len(), if added { 3 } else { 2 });
            assert_eq!(
                messages
                    .last()
                    .and_then(|message| message.get("role"))
                    .and_then(Value::as_str),
                Some(if added {
                    "user"
                } else {
                    messages[1].get("role").and_then(Value::as_str).unwrap()
                })
            );
        }
    }
}
