//! What the interface does as processes start and end: the wake, the footer's
//! count, the completion row and the idle continuation.

use crate::json::Value;
use crate::tui::{App, Feedback, Kind, Worker, activity, state::Item, text};

impl App {
    /// A process starting or ending wakes the loop, also when nothing else would.
    pub(in crate::tui) fn watch_processes(&self) {
        if let Some(terminal) = &self.terminal {
            let waker = terminal.waker();
            self.process_observer
                .on_change(std::sync::Arc::new(move || waker.wake()));
        }
    }

    /// Shows completions and the footer's count. Reads the observer's own
    /// position, so it never takes anything from the model's delivery.
    pub(in crate::tui) fn poll_processes(&mut self) -> bool {
        self.sync_process_observer();
        let (cursor, completions) = self.process_observer.completions_since(self.process_cursor);
        self.process_cursor = cursor;
        let mut dirty = !completions.is_empty();
        for completion in completions {
            self.show_process_completion(&completion);
            if let Err(error) = self.save_process_completion(&completion) {
                self.warn(&format!("Process result could not be saved: {error}"));
            }
        }
        let background = self.process_observer.background_count();
        if self.state.background_processes != background {
            self.state.background_processes = background;
            dirty = true;
        }
        dirty
    }

    /// A row at its position in the conversation and a notice on the status
    /// row, loud only when the process failed by itself.
    fn show_process_completion(&mut self, completion: &crate::tools::processes::Completion) {
        self.state.process_completion(completion.value());
        let calm = completion.stop_requested
            || completion.status == "exited" && completion.exit_code == Some(0);
        let kind = if calm { Kind::Notice } else { Kind::Warning };
        self.state.notify(Feedback::result(
            kind,
            format!(
                "Process · {} · {}",
                completion.label(),
                text::clean(&completion.command).replace('\n', " ")
            ),
        ));
    }

    /// Save the visible row independently of delivery to the model. The
    /// process registry keeps its separate `reported` flag until acceptance.
    fn save_process_completion(
        &self,
        completion: &crate::tools::processes::Completion,
    ) -> Result<(), String> {
        let mut event = completion.value();
        let Value::Object(fields) = &mut event else {
            unreachable!("a process completion is an object")
        };
        fields.insert("type".into(), Value::string("process_finished"));
        // A receipt can be in the live conversation before the worker's
        // checkpoint. The durable event must retain the row it followed.
        if let Some(at) = self.state.items.iter().position(|item| {
            matches!(item, Item::Tool { id, .. } if id == &format!("process:{}", completion.id))
        }) && let Some(id) = self.state.items[..at].iter().rev().find_map(|item| match item {
            Item::Tool { id, .. } if !id.starts_with("process:") => Some(id),
            _ => None,
        }) {
            fields.insert("after_tool".into(), Value::string(id));
        }
        fields.insert(
            "after_message".into(),
            Value::number(self.archive.messages.lock().unwrap().len()),
        );
        if let Some(handle) = &self.persistence {
            handle.append_event(&self.archive, event)?;
        } else {
            let mut events = self.archive.events.lock().unwrap();
            if !events.iter().any(|saved| {
                saved.get("type").and_then(Value::as_str) == Some("process_finished")
                    && saved.get("process_id").and_then(Value::as_str)
                        == Some(completion.id.as_str())
            }) {
                events.push(event);
            }
        }
        Ok(())
    }

    /// Continues an idle conversation when a process ended by itself. The
    /// user's explicit stop, a failed turn, or anything waiting for the user
    /// holds it back; the next request carries the completion either way.
    pub(in crate::tui) fn start_process_continuation(&mut self) -> bool {
        if !self.process_auto_resume
            || !self.idle()
            || !self.state.queue.is_empty()
            || self.state.selector.is_some()
            || self.state.quitting.is_some()
            || self.state.question.is_some()
            || self.state.permission.is_some()
            || self
                .agent
                .as_ref()
                .is_none_or(|agent| !agent.can_continue_processes())
        {
            return false;
        }
        let agent = self.hold();
        self.state.begin_tool_turn();
        self.state.activity = Some(activity::Activity::new());
        self.state.backend_failure = None;
        self.state.status = "Reviewing process results…".into();
        self.run_worker(Worker::continue_processes(agent));
        true
    }
}
