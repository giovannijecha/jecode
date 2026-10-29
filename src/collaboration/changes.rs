//! A collaborator's file changes with their hunks. Receipts keep only line
//! counts; the hunks stay in the child's own session, the one copy, read
//! live while it runs and from its saved journal afterwards.

use super::Observer;
use crate::{file_change::Change, json::Value, sessions::Store};
use std::path::Path;

impl Observer {
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
            .map(|handle| {
                handle
                    .store()
                    .bucket()
                    .join("collaboration")
                    .join(handle.id())
                    .join("child-home")
            });
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
            Some((
                event.get("tool_call_id")?.as_str()?.to_owned(),
                Change::from_value(event.get("change")?).ok()?,
            ))
        })
        .collect()
}
