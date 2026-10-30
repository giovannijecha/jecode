//! A short session title, generated once from the first request.
use super::Agent;
use crate::{attachments::Prompt, json::Value};

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
        let Some(handle) = self.persistence.clone() else {
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
        std::thread::spawn(move || {
            let title = client.prepare_turn().ok().and_then(|_| {
                client
                    .sample(&messages, false, None, None, &mut |_| Ok(()))
                    .ok()
            });
            if let Some(title) = title.and_then(|completion| clean(&completion.text)) {
                let _ = handle.name(&title);
            }
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
