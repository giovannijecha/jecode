use super::{App, Feedback, Kind, activity, feedback::Ending};
use crate::{attachments::Prompt, backend::FailureReason, events::Event};
use std::time::Instant;

impl App {
    pub(super) fn poll(&mut self) -> Result<bool, String> {
        let mut dirty = self.state.expire_feedback(Instant::now());
        dirty |= self.poll_auth_updates();
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
                    self.state.backend_failure = None;
                    self.submission = None;
                    self.state.status = "Ready".into();
                    self.state.clear_progress();
                    if let Some(activity) = activity {
                        // What responses reported, never the routing's intent.
                        let served = agent
                            .charges()
                            .describe()
                            .map(|served| agent.redact(&served));
                        let facts = [
                            Some(activity::duration(activity.elapsed())),
                            activity::tools(activity.tools),
                            activity::generated(activity.generated),
                            served,
                        ];
                        self.state
                            .notify(Feedback::ending(Ending::Done, joined(facts)));
                    }
                }
                result => {
                    let error = result.err().unwrap_or_else(|| "Turn interrupted".into());
                    self.state.queue.pause();
                    self.state.clear_progress();
                    let returned = self.return_unanswered(&mut agent);
                    let backend_failure = self.state.backend_failure.take();
                    let key = !cancelled
                        && matches!(
                            backend_failure.as_ref(),
                            Some((FailureReason::RejectedKey, _))
                        );
                    if !cancelled {
                        let diagnostic = backend_failure
                            .as_ref()
                            .map_or(error.as_str(), |(_, diagnostic)| diagnostic.as_str());
                        self.state.diagnostic(&agent.redact(diagnostic));
                    }
                    let paused = self.state.queue.len();
                    let facts = [
                        activity
                            .as_ref()
                            .map(|activity| activity::duration(activity.elapsed())),
                        if returned {
                            Some("prompt restored".into())
                        } else if cancelled {
                            Some("your next message continues from here".into())
                        } else {
                            None
                        },
                        self.save_error.clone().filter(|_| returned),
                        (paused > 0).then(|| {
                            format!(
                                "{paused} {} paused",
                                if paused == 1 { "draft" } else { "drafts" }
                            )
                        }),
                    ];
                    self.state.status = "Turn interrupted".into();
                    let mut notice = Feedback::ending(
                        if cancelled {
                            Ending::Stopped
                        } else {
                            Ending::Failed
                        },
                        joined(facts),
                    );
                    if key {
                        notice = notice.action("/settings", "update key");
                    } else if paused > 0 {
                        notice = notice.action("Alt+↑", "review");
                    }
                    self.state.notify(notice);
                    self.refresh_draft_menu();
                    self.edited();
                }
            }
            self.archive = agent.archive();
            self.agent = Some(agent);
            self.state.finish_tool_turn();
            self.apply_deferred();
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
        dirty |= self.refresh_agents();
        dirty |= self.refresh_question();
        dirty |= self.refresh_permission();
        dirty |= self.refresh_todo();
        dirty |= self.start_collaboration_continuation();
        Ok(dirty)
    }
}

/// The facts present, joined as the status row reads them.
fn joined<const N: usize>(facts: [Option<String>; N]) -> String {
    facts.into_iter().flatten().collect::<Vec<_>>().join(" · ")
}

impl App {
    fn next_event(&self) -> Option<Event> {
        self.worker.as_ref()?.events.try_recv().ok()
    }

    pub(super) fn worker_event(&mut self, event: Event) {
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
            Event::Collaboration { event } => {
                self.show_collaboration(event);
            }
            Event::UserQuestion { request } => {
                if self.state.question.is_none() {
                    self.make_room_for_ask();
                }
                let arriving = self
                    .state
                    .question
                    .as_ref()
                    .is_none_or(|current| current.id != request.id);
                self.state.event(Event::UserQuestion { request });
                if arriving {
                    self.guard_ask_arrival();
                }
            }
            Event::ToolStarted {
                id,
                name,
                arguments,
            } if name == "agent" && super::agents::receipt_replaces_tool(&arguments) => {
                if let Some(activity) = &mut self.state.activity {
                    activity.tools += 1;
                }
                self.collaboration_receipt_calls.insert(id);
            }
            Event::ToolFinished {
                id,
                name,
                summary,
                result,
                ..
            } if name == "agent" && self.collaboration_receipt_calls.remove(&id) => {
                if result
                    .get("error")
                    .and_then(crate::json::Value::as_str)
                    .is_some()
                {
                    self.state.diagnostic(&self.archive.redactor.text(&summary));
                }
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
