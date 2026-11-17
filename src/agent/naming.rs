//! A short session title, generated once from the first request.
use super::{Agent, timing::RequestTiming};
use crate::{
    attachments::Prompt,
    backend::{Backend, Update, Usage},
    cancel::Cancellation,
    effort::Effort,
    events::EventSink,
    json::Value,
};
use std::{thread::JoinHandle, time::Instant};

const PROMPT: &str = "Name this coding session from the user's first request. Reply with the title only: two to six words in the language of the request, without quotes or a final period.";

/// The request text the model reads; the rest adds cost, not a better name.
const REQUEST_LIMIT: usize = 2000;

pub(super) struct Task {
    cancellation: Cancellation,
    worker: Option<JoinHandle<Result<(), String>>>,
}

impl Task {
    fn join(&mut self) -> Result<(), String> {
        self.worker.take().map_or(Ok(()), |worker| {
            worker
                .join()
                .map_err(|_| "Session title worker failed".to_owned())?
        })
    }
}

impl Drop for Task {
    fn drop(&mut self) {
        self.cancellation.cancel();
        let _ = self.join();
    }
}

impl Agent {
    /// The application names its sessions; collaborators and fixtures never do.
    pub fn name_sessions(&mut self) {
        self.naming = true;
    }

    /// A one-shot owns all title work until its result reaches the journal,
    /// including when the main request or its output fails.
    pub fn run_single_turn(
        &mut self,
        prompt: impl Into<Prompt>,
        events: &mut impl EventSink,
    ) -> Result<(), String> {
        self.set_single_run();
        let result = self.run_turn(prompt, events);
        let naming = self.finish_naming();
        finish_result(result, naming)
    }

    fn finish_naming(&mut self) -> Result<(), String> {
        self.naming_task
            .take()
            .map_or(Ok(()), |mut task| task.join())
    }

    pub(super) fn stop_naming(&mut self) -> Result<(), String> {
        if let Some(task) = &self.naming_task {
            task.cancellation.cancel();
        }
        self.finish_naming()
    }

    /// Names the session in the background after its first request is saved.
    /// A failed request keeps the title taken from the request itself.
    pub(super) fn name_session(&mut self, prompt: &Prompt, first: bool) -> Result<(), String> {
        let Some(handle) = self.persistence.as_ref() else {
            return Ok(());
        };
        if !self.naming || self.child || !first || handle.titled() {
            return Ok(());
        }
        let cancellation = self.cancellation.child();
        let mut client = match self
            .client
            .services()
            .with_cancellation(cancellation.clone())
            .create(self.client.model().into())
        {
            Ok(client) => client,
            Err(error) => return self.naming_failure(&error),
        };
        client.set_cancellation(cancellation.clone());
        client.clone_routing_from(self.client.as_ref());
        client.set_routing(self.client.routing());
        let archive = self.archive();
        let name = handle.namer(archive.messages, archive.events);
        let request = prompt
            .display()
            .chars()
            .take(REQUEST_LIMIT)
            .collect::<String>();
        let messages = [
            Value::object([
                ("role", Value::string("system")),
                ("content", Value::string(PROMPT)),
            ]),
            Value::object([
                ("role", Value::string("user")),
                ("content", Value::string(request)),
            ]),
        ];
        let token = cancellation.clone();
        match std::thread::Builder::new()
            .name("jecode-session-title".into())
            .spawn(move || {
                let (title, event, panicked) = generate(client.as_mut(), &messages, &token);
                name(title.as_deref(), event)?;
                if panicked {
                    Err("Session title worker failed".into())
                } else {
                    Ok(())
                }
            }) {
            Ok(worker) => {
                self.naming_task = Some(Task {
                    cancellation,
                    worker: Some(worker),
                });
                Ok(())
            }
            Err(error) => self.naming_failure(&format!("Could not start session title: {error}")),
        }
    }

    fn naming_failure(&self, error: &str) -> Result<(), String> {
        let event = RequestTiming::start().event(
            "session_title",
            (self.backend_id(), self.model(), Effort::Low),
            1,
            "failed",
            None,
            0,
        );
        let archive = self.archive();
        self.persistence
            .as_ref()
            .unwrap()
            .namer(archive.messages, archive.events)(None, with_error(event, error))
    }
}

pub(super) fn finish_result(
    result: Result<(), String>,
    naming: Result<(), String>,
) -> Result<(), String> {
    match (result, naming) {
        (Err(error), Err(naming)) => Err(format!("{error}\n{naming}")),
        (result, naming) => result.and(naming),
    }
}

fn generate(
    client: &mut dyn Backend,
    messages: &[Value],
    cancellation: &Cancellation,
) -> (Option<String>, Value, bool) {
    let mut timing = RequestTiming::start();
    let mut usage = Usage::default();
    let mut has_usage = false;
    let mut effort = Effort::Low;
    let attempt = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        if cancellation.requested() {
            return Err("Operation cancelled".into());
        }
        client.prepare_turn().and_then(|_| {
            effort = client.maintenance_effort(Effort::Low);
            client.sample_with_effort(effort, messages, false, Some(512), None, &mut |update| {
                match update {
                    Update::Waiting { request_bytes } => {
                        timing.request_bytes = Some(request_bytes);
                        timing.dispatched = Some(Instant::now());
                    }
                    Update::Usage(snapshot) => {
                        has_usage = true;
                        usage.update(snapshot);
                    }
                    Update::Transport(name) => timing.transport = Some(name),
                    Update::TextDelta(_) | Update::Reasoning | Update::Working => {
                        timing.first_update.get_or_insert_with(Instant::now);
                    }
                }
                if cancellation.requested() {
                    Err("Operation cancelled".into())
                } else {
                    Ok(())
                }
            })
        })
    }));
    let panicked = attempt.is_err();
    let result = attempt.unwrap_or_else(|_| Err("Session title worker failed".into()));
    let title = result.as_ref().ok().and_then(|completion| {
        if let Some(snapshot) = completion.usage {
            has_usage = true;
            usage.update(snapshot);
        }
        clean(&completion.text)
    });
    let event = timing.event(
        "session_title",
        (client.id(), client.model(), effort),
        1,
        if result.is_ok() {
            "complete"
        } else if cancellation.requested() {
            "cancelled"
        } else {
            "failed"
        },
        has_usage.then_some(usage),
        0,
    );
    let event = match result {
        Ok(_) => event,
        Err(error) => with_error(event, &error.message),
    };
    (title, event, panicked)
}

fn with_error(mut event: Value, error: &str) -> Value {
    if let Value::Object(fields) = &mut event {
        fields.insert("error".into(), Value::string(error));
    }
    event
}

/// The first line of the reply, without wrapping quotes, emphasis, heading
/// marks or a final period.
pub(super) fn clean(reply: &str) -> Option<String> {
    let line = reply.lines().map(str::trim).find(|line| !line.is_empty())?;
    let line = line.split_whitespace().collect::<Vec<_>>().join(" ");
    let title = line
        .trim_matches(|c: char| {
            c.is_whitespace() || matches!(c, '"' | '\'' | '`' | '*' | '#' | '“' | '”' | '«' | '»')
        })
        .trim_end_matches('.')
        .trim_end();
    let title = title
        .chars()
        .filter(|c| !c.is_control())
        .take(80)
        .collect::<String>();
    (!title.is_empty()).then_some(title)
}
