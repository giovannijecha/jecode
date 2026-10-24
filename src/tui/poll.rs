use super::{App, Feedback, Kind, activity};
use crate::{attachments::Prompt, events::Event};
use std::time::Instant;

impl App {
    pub(super) fn poll(&mut self) -> Result<bool, String> {
        let mut dirty = self.state.expire_feedback(Instant::now());
        dirty |= self.poll_job()?;
        dirty |= self.poll_imports();
        dirty |= self.finish_delete(false)?;
        dirty |= self.finish_checkpoints(false);
        for _ in 0..32 {
            let Some(event) = self.next_event() else {
                break;
            };
            self.worker_event(event);
            dirty = true;
        }
        if self.worker.as_ref().is_some_and(|worker| worker.finished()) {
            while let Some(event) = self.next_event() {
                self.worker_event(event);
            }
            let (mut agent, result) = self.worker.take().unwrap().finish()?;
            self.reconcile_accepted();
            self.state.settle_tools(&agent.archive());
            self.state.finish_stream();
            self.state.close_tools();
            let activity = self.state.activity.take();
            let cancelled = activity.as_ref().is_some_and(|activity| activity.stopping);
            match result {
                Ok(()) if !cancelled => {
                    self.submission = None;
                    self.state.status = "Ready".into();
                    self.state.clear_progress();
                    if let Some(activity) = activity {
                        // What responses reported, never the routing's intent.
                        let served = agent
                            .charges()
                            .describe()
                            .map(|served| format!(" · {}", agent.redact(&served)))
                            .unwrap_or_default();
                        self.state.notify(Feedback::result(
                            Kind::Notice,
                            format!(
                                "Complete · {} · {} {}{served}",
                                activity::duration(activity.started.elapsed()),
                                activity.tools,
                                if activity.tools == 1 { "tool" } else { "tools" }
                            ),
                        ));
                    }
                }
                result => {
                    let error = result.err().unwrap_or_else(|| "Turn interrupted".into());
                    self.state.queue.pause();
                    let returned = self.return_unanswered(&mut agent);
                    let mut message = if cancelled {
                        "Interrupted".into()
                    } else {
                        agent.redact(&error)
                    };
                    if returned {
                        message.push_str(" · prompt restored to composer");
                        if let Some(error) = &self.save_error {
                            message.push_str(&format!(" · {error}"));
                        }
                    }
                    if !self.state.queue.is_empty() {
                        message.push_str(" · pending drafts paused · Alt+↑ to review");
                    }
                    self.state.status = "Turn interrupted".into();
                    self.state.notify(Feedback::result(
                        if cancelled {
                            Kind::Warning
                        } else {
                            Kind::Error
                        },
                        message,
                    ));
                    self.refresh_draft_menu();
                    self.edited();
                }
            }
            self.archive = agent.archive();
            self.agent = Some(agent);
            self.state.finish_tool_turn();
            self.refresh_context();
            dirty = true;
        }
        dirty |= self.poll_copy();
        while self.idle()
            && self.state.selector.is_none()
            && self.state.information.is_none()
            && !self.state.queue.is_editing()
        {
            let Some(prompt) = self.state.queue.pop_front() else {
                break;
            };
            self.exit_requested = self.dispatch(prompt)?;
            dirty = true;
            if self.exit_requested {
                break;
            }
        }
        dirty |= self.refresh_processes(false);
        Ok(dirty)
    }

    fn next_event(&self) -> Option<Event> {
        self.worker.as_ref()?.events.try_recv().ok()
    }

    fn worker_event(&mut self, event: Event) {
        if let Some(submission) = &mut self.submission {
            submission.observe(&event);
        }
        match event {
            Event::Accepted { prompts } => {
                let count = prompts.len();
                self.state
                    .queue
                    .inbox()
                    .with(|entries| entries.acknowledge(count));
                self.show_accepted(prompts);
            }
            event => self.state.event(event),
        }
    }

    /// Receipts remain in the inbox until the interface displays them. This
    /// also handles a worker whose acceptance event was never delivered.
    pub(super) fn reconcile_accepted(&mut self) {
        let missed = self
            .state
            .queue
            .inbox()
            .with(|entries| entries.unacknowledged());
        self.show_accepted(missed);
    }

    /// Shows messages the agent took into the active turn after the output
    /// that preceded them, as sent prompts.
    fn show_accepted(&mut self, prompts: Vec<Prompt>) {
        if prompts.is_empty() {
            return;
        }
        self.state.finish_stream();
        self.state.close_tools();
        for prompt in &prompts {
            if let Some(submission) = &mut self.submission {
                submission.accepted(prompt.clone(), self.state.items.len());
            }
            self.state.history.record(prompt);
            self.state
                .message(Kind::User, &self.archive.redactor.text(&prompt.display()));
        }
        self.refresh_draft_menu();
        self.capture_input();
    }
}
