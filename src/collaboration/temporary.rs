//! Temporary files a main agent reads from its collaborators' own areas.

use super::Observer;
use crate::json::Value;
use std::path::PathBuf;

impl Observer {
    /// The collaborators of this conversation whose temporary areas hold
    /// `alias`, in the order they started, each with the area and the file;
    /// only `agent` when one is named. Lookups are read-only.
    pub fn temporary_files(
        &self,
        agent: Option<&str>,
        alias: &str,
    ) -> Result<Vec<(String, PathBuf, PathBuf)>, String> {
        let parent = self
            .0
            .parent
            .lock()
            .unwrap()
            .as_ref()
            .and_then(|(_, handle)| handle.clone());
        let Some(handle) = parent else {
            return Ok(Vec::new());
        };
        let store = handle.store();
        let home = store.collaborator_home(&handle.id()).join("tmp");
        let mut found = Vec::new();
        for (id, session) in self.sessions() {
            if agent.is_some_and(|agent| agent != id) {
                continue;
            }
            if let Some((root, path)) =
                crate::scratch::find(store.bucket(), &home, &session, alias)?
            {
                found.push((id, root, path));
            }
        }
        Ok(found)
    }

    /// Each collaborator's ID and session, including closed ones whose
    /// records only the saved receipts still hold.
    fn sessions(&self) -> Vec<(String, String)> {
        let state = self.0.state.lock().unwrap();
        let saved = state
            .receipts
            .iter()
            .filter_map(|receipt| receipt.get("records").and_then(Value::as_array))
            .flatten()
            .filter_map(|record| {
                Some((
                    record.get("id")?.as_str()?.to_owned(),
                    record.get("child_session")?.as_str()?.to_owned(),
                ))
            });
        let live = state
            .records
            .iter()
            .filter_map(|record| Some((record.id.clone(), record.child_session.clone()?)));
        let mut sessions = Vec::new();
        for pair in saved.chain(live) {
            if !sessions.contains(&pair) {
                sessions.push(pair);
            }
        }
        sessions
    }
}
