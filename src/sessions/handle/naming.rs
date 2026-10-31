use super::{Handle, Live};
use crate::{export::Messages, json::Value};
use std::{sync::Arc, time::Instant};

impl Handle {
    /// Whether the session already has a generated title.
    pub fn titled(&self) -> bool {
        self.0.live.lock().unwrap().document.title.is_some()
    }

    /// The naming request may outlive the active session. Its writer must not
    /// keep the session lease while the provider is still responding.
    pub(crate) fn namer(
        &self,
        messages: Messages,
        events: Messages,
    ) -> impl FnOnce(Option<&str>, Value) + Send + 'static {
        let shared = Arc::downgrade(&self.0);
        let messages = Arc::downgrade(&messages);
        let events = Arc::downgrade(&events);
        move |title, event| {
            if let Some(shared) = shared.upgrade() {
                let _ = Handle(shared).naming_result(
                    title,
                    event,
                    messages.upgrade(),
                    events.upgrade(),
                );
            }
        }
    }

    /// Save a background naming attempt with the title. When the active agent
    /// still owns this history, keep its archive aligned with the journal.
    fn naming_result(
        &self,
        title: Option<&str>,
        mut event: Value,
        messages: Option<Messages>,
        events: Option<Messages>,
    ) -> Result<(), String> {
        let _io = self.0.io.lock().unwrap();
        let messages = messages.as_ref().map(|messages| messages.lock().unwrap());
        let mut events = events.as_ref().map(|events| events.lock().unwrap());
        let mut live = self.0.live.lock().unwrap();
        if live.deleted {
            return Ok(());
        }
        let linked = messages
            .as_ref()
            .zip(events.as_ref())
            .is_some_and(|(messages, events)| {
                messages.len() >= live.document.messages.len()
                    && events.len() >= live.document.events.len()
                    && messages
                        .iter()
                        .zip(&live.document.messages)
                        .all(|(current, saved)| live.redactor.value(current) == *saved)
                    && events
                        .iter()
                        .zip(&live.document.events)
                        .all(|(current, saved)| live.redactor.value(current) == *saved)
            });
        if linked {
            let messages = messages.as_ref().unwrap();
            let events = events.as_mut().unwrap();
            let new_messages = messages
                .iter()
                .skip(live.document.messages.len())
                .map(|message| live.redactor.value(message))
                .collect::<Vec<_>>();
            live.document.messages.extend(new_messages);
            let new_events = events
                .iter()
                .skip(live.document.events.len())
                .map(|event| live.redactor.value(event))
                .collect::<Vec<_>>();
            live.document.events.extend(new_events);
        }
        if let Value::Object(fields) = &mut event {
            fields.insert(
                "after_message".into(),
                Value::number(live.document.messages.len()),
            );
        }
        if linked {
            events.as_mut().unwrap().push(event.clone());
        }
        let event = live.redactor.value(&event);
        live.document.events.push(event);
        if let Some(title) = title {
            set_title(&mut live, title);
        }
        live.dirty.get_or_insert_with(Instant::now);
        live.revision = live.revision.wrapping_add(1);
        drop(live);
        drop(events);
        drop(messages);
        self.save_locked_io()
    }

    /// Saves the first generated title; later ones never replace it.
    #[cfg(test)]
    pub fn name(&self, title: &str) -> Result<(), String> {
        let mut live = self.0.live.lock().unwrap();
        if live.deleted || !set_title(&mut live, title) {
            return Ok(());
        }
        live.dirty.get_or_insert_with(Instant::now);
        live.revision = live.revision.wrapping_add(1);
        drop(live);
        self.flush()
    }
}

fn set_title(live: &mut Live, title: &str) -> bool {
    if live.document.title.is_some() {
        return false;
    }
    // Redaction may lengthen it; the saved title keeps the parse limit.
    let title = live.redactor.text(title);
    let title: String = title
        .chars()
        .filter(|c| !c.is_control())
        .take(100)
        .collect();
    let title = title.trim();
    if title.is_empty() {
        return false;
    }
    live.document.title = Some(title.into());
    true
}
