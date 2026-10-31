//! A short session title, generated once from the first request.
use super::{Agent, timing::RequestTiming};
use crate::{
    attachments::Prompt,
    backend::{Update, Usage},
    effort::Effort,
    json::Value,
};
use std::time::Instant;

const PROMPT: &str = "Name this coding session from the user's first request. Reply with the title only: two to six words in the language of the request, without quotes or a final period.";

/// The request text the model reads; the rest adds cost, not a better name.
const REQUEST_LIMIT: usize = 2000;

impl Agent {
    /// The application names its sessions; collaborators and fixtures never do.
    pub fn name_sessions(&mut self) {
        self.naming = true;
    }

    /// Names the session in the background after its first request is saved.
    /// A failed request keeps the title taken from the request itself.
    pub(super) fn name_session(&self, prompt: &Prompt, first: bool) {
        let Some(handle) = self.persistence.as_ref() else {
            return;
        };
        if !self.naming || self.child || !first || handle.titled() {
            return;
        }
        let Ok(mut client) = self.client.services().create(self.client.model().into()) else {
            return;
        };
        client.clone_routing_from(self.client.as_ref());
        client.set_routing(self.client.routing());
        let archive = self.archive();
        let name = handle.namer(archive.messages, archive.events);
        let backend = client.id().to_owned();
        let model = self.redact(client.model());
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
        let _ = std::thread::Builder::new()
            .name("jecode-session-title".into())
            .spawn(move || {
                let mut timing = RequestTiming::start();
                let mut usage = Usage::default();
                let mut has_usage = false;
                let mut effort = Effort::Low;
                let result = client.prepare_turn().and_then(|_| {
                    effort = client.maintenance_effort(Effort::Low);
                    client.sample_with_effort(
                        effort,
                        &messages,
                        false,
                        Some(512),
                        None,
                        &mut |update| {
                            match update {
                                Update::Waiting { request_bytes } => {
                                    timing.request_bytes = Some(request_bytes);
                                    timing.dispatched = Some(Instant::now());
                                }
                                Update::Usage(snapshot) => {
                                    has_usage = true;
                                    usage.update(snapshot);
                                }
                                Update::TextDelta(_) | Update::Reasoning | Update::Working => {
                                    timing.first_update.get_or_insert_with(Instant::now);
                                }
                            }
                            Ok(())
                        },
                    )
                });
                let title = result.as_ref().ok().and_then(|completion| {
                    if let Some(snapshot) = completion.usage {
                        has_usage = true;
                        usage.update(snapshot);
                    }
                    clean(&completion.text)
                });
                let event = timing.event(
                    "session_title",
                    (&backend, &model, effort),
                    1,
                    if result.is_ok() { "complete" } else { "failed" },
                    has_usage.then_some(usage),
                    0,
                );
                name(title.as_deref(), event);
            });
    }
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
