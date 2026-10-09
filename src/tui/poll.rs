use super::{App, Feedback, Kind, activity};
use std::time::Instant;

impl App {
    pub(super) fn poll(&mut self) -> Result<bool, String> {
        let mut dirty = self.state.expire_feedback(Instant::now());
        dirty |= self.poll_job()?;
        dirty |= self.poll_imports();
        dirty |= self.finish_delete(false)?;
        if let Some(worker) = &self.worker {
            for _ in 0..32 {
                match worker.events.try_recv() {
                    Ok(event) => {
                        self.state.event(event);
                        dirty = true;
                    }
                    Err(_) => break,
                }
            }
            if worker.finished() {
                while let Ok(event) = worker.events.try_recv() {
                    self.state.event(event);
                }
                let (agent, result) = self.worker.take().unwrap().finish()?;
                self.state.settle_tools(&agent.archive());
                self.state.finish_stream();
                self.state.close_tools();
                let activity = self.state.activity.take();
                let cancelled = activity.as_ref().is_some_and(|activity| activity.stopping);
                match result {
                    Ok(()) if !cancelled => {
                        self.state.status = "Ready".into();
                        self.state.clear_progress();
                        if let Some(activity) = activity {
                            self.state.notify(Feedback::result(
                                Kind::Notice,
                                format!(
                                    "Complete · {} · {} {}",
                                    activity::duration(activity.started.elapsed()),
                                    activity.tools,
                                    if activity.tools == 1 { "tool" } else { "tools" }
                                ),
                            ));
                        }
                    }
                    result => {
                        let error = result.err().unwrap_or_else(|| "Turn interrupted".into());
                        self.state.status = "Turn interrupted".into();
                        self.state.notify(Feedback::result(
                            if cancelled {
                                Kind::Warning
                            } else {
                                Kind::Error
                            },
                            if cancelled {
                                "Interrupted · pending drafts paused · Alt+↑ to review".into()
                            } else if error.contains("HTTP 401") {
                                "Account disconnected · /settings to update the OpenRouter key"
                                    .into()
                            } else {
                                format!(
                                    "{} · pending drafts paused · Alt+↑ to review",
                                    agent.redact(&error)
                                )
                            },
                        ));
                        self.state.queue.pause();
                        self.refresh_draft_menu();
                        self.edited();
                    }
                }
                self.archive = agent.archive();
                self.agent = Some(agent);
                dirty = true;
            }
        }
        dirty |= self.poll_copy();
        while self.worker.is_none()
            && self.job.is_none()
            && self.deletion_job.is_none()
            && self.state.selector.is_none()
            && self.state.information.is_none()
            && !self.state.queue.is_editing()
        {
            let Some(prompt) = self.state.queue.messages.pop_front() else {
                break;
            };
            self.exit_requested = self.dispatch(prompt)?;
            dirty = true;
            if self.exit_requested {
                break;
            }
        }
        Ok(dirty)
    }
}
