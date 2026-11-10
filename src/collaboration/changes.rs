//! A collaborator's file changes with their hunks. Receipts keep only line
//! counts; the hunks stay in the child's own session, the one copy, read
//! live while it runs and from its saved journal afterwards.

use super::Observer;
use crate::{file_change::Change, json::Value, sessions::Store};
use std::path::Path;

impl Observer {
    /// Live session event count, used to notice changes saved after a step's
    /// card already reported its line counts.
    pub fn change_revision(&self, id: &str) -> Option<usize> {
        let events = {
            let state = self.0.state.lock().unwrap();
            state
                .records
                .iter()
                .find(|record| record.id == id)?
                .session_events
                .clone()
        };
        events.map(|events| events.lock().unwrap().len())
    }

    #[cfg(test)]
    pub(crate) fn fixture_change_stream(&self, id: &str, events: crate::export::Messages) {
        let seed = Value::object([
            ("id", Value::string(id)),
            ("number", Value::number(1)),
            ("state", Value::string("returned")),
            ("workspace_closed", Value::Bool(true)),
            ("log", Value::Array(vec![])),
        ]);
        let mut record = super::Record::from_value(&seed).unwrap();
        record.session_events = Some(events);
        self.0
            .state
            .lock()
            .unwrap()
            .records
            .push(super::Tracked::new(record));
    }

    /// The collaborator's changed calls in order, by call id. Empty when its
    /// session cannot be read, which leaves its cards with counts only.
    pub fn changes(&self, id: &str) -> Vec<(String, Change)> {
        let (live, session, directory) = {
            let state = self.0.state.lock().unwrap();
            let Some(record) = state.records.iter().find(|record| record.id == id) else {
                return Vec::new();
            };
            (
                record.session_events.clone(),
                record.child_session.clone(),
                record.directory.clone(),
            )
        };
        if let Some(events) = live {
            return changes(&events.lock().unwrap());
        }
        let Some(session) = session else {
            return Vec::new();
        };
        let home = self
            .0
            .parent
            .lock()
            .unwrap()
            .as_ref()
            .and_then(|(_, handle)| handle.as_ref())
            .map(|handle| handle.store().collaborator_home(&handle.id()));
        home.and_then(|home| Store::new(home, Path::new(&directory)).ok())
            .and_then(|store| store.read(&session).ok())
            .map_or_else(Vec::new, |document| changes(&document.events))
    }
}

fn changes(events: &[Value]) -> Vec<(String, Change)> {
    events
        .iter()
        .filter(|event| event.get("type").and_then(Value::as_str) == Some("tool_change"))
        .filter_map(|event| {
            // Cards know a `steps` call by its steps, each with its own change.
            let id = event.get("step").or_else(|| event.get("tool_call_id"))?;
            Some((
                id.as_str()?.to_owned(),
                Change::from_value(event.get("change")?).ok()?,
            ))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_change_inside_a_steps_call_belongs_to_its_step() {
        let change = Change::between(None, "text\n").unwrap();
        let event = |step: Option<&str>| {
            let mut event = Value::object([
                ("type", Value::string("tool_change")),
                ("tool_call_id", Value::string("call")),
                ("change", change.value()),
            ]);
            if let (Some(step), Value::Object(fields)) = (step, &mut event) {
                fields.insert("step".into(), Value::string(step));
            }
            event
        };
        let found = changes(&[event(None), event(Some("call_s1")), event(Some("call_s2"))]);
        let ids: Vec<_> = found.iter().map(|(id, _)| id.as_str()).collect();
        assert_eq!(ids, ["call", "call_s1", "call_s2"]);
    }
}
