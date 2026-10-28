//! Restoring a session's collaborators when the observer binds to it.
use super::{Observer, Record, workspace::Workspace};
use crate::{export::Archive, json::Value, sessions::Handle};
use std::fs;

impl Observer {
    /// Restores open assignments from the latest saved receipt, marks
    /// interrupted work unknown and reclaims isolated copies no saved record
    /// owns. Returns a note when such copies were removed or left in place.
    pub fn bind(&self, archive: Archive, handle: Option<Handle>) -> Result<Option<String>, String> {
        let session = handle.clone();
        let mut state = self.0.state.lock().unwrap();
        let mut parent = self.0.parent.lock().unwrap();
        *parent = Some((archive.clone(), handle));
        state.records.clear();
        state.receipts.clear();
        state.last_number = 0;
        let mut latest = None;
        for event in archive.events.lock().unwrap().iter() {
            if event.get("type").and_then(Value::as_str) == Some("collaboration") {
                state.receipts.push(event.clone());
                state.last_number = state
                    .last_number
                    .max(event.get("number").and_then(Value::as_usize).unwrap_or(0));
                if let Some(records) = event.get("records").and_then(Value::as_array) {
                    for record in records {
                        state.last_number = state
                            .last_number
                            .max(record.get("number").and_then(Value::as_usize).unwrap_or(0));
                    }
                }
                latest = event
                    .get("records")
                    .and_then(Value::as_array)
                    .map(<[Value]>::to_vec);
            }
        }
        let mut interrupted = Vec::new();
        if let Some(latest) = latest {
            interrupted = latest
                .iter()
                .filter(|record| {
                    matches!(
                        record.get("state").and_then(Value::as_str),
                        Some("running" | "stopping")
                    )
                })
                .filter_map(|record| record.get("id").and_then(Value::as_str).map(str::to_owned))
                .collect();
            state.records = latest
                .iter()
                .filter(|record| record.get("workspace_closed") != Some(&Value::Bool(true)))
                .map(Record::from_value)
                .collect::<Result<_, _>>()?;
        }
        let owned = state
            .records
            .iter()
            .map(|record| record.id.clone())
            .collect::<Vec<_>>();
        let unknown = state
            .records
            .iter()
            .filter(|record| interrupted.contains(&record.id))
            .map(|record| {
                (
                    record.id.clone(),
                    record.received_version,
                    record.title.clone(),
                )
            })
            .collect::<Vec<_>>();
        drop(state);
        drop(parent);
        for (id, version, title) in unknown {
            self.receipt(
                "unknown",
                Some(&id),
                version,
                &title,
                vec!["Previous collaborator execution has no confirmed result".into()],
            )?;
        }
        Ok(session.and_then(|handle| self.reclaim(&handle, &owned)))
    }

    /// An isolated copy is prepared before the `created` receipt that owns it
    /// is saved, so a stop in between leaves a copy that no record names.
    /// Only this session's own storage is examined, and only under its
    /// exclusive lease: no other live conversation can own a copy there, and
    /// one that no restored record names has no owner.
    fn reclaim(&self, handle: &Handle, owned: &[String]) -> Option<String> {
        if !handle.leased() {
            return None;
        }
        let store = handle.store();
        let storage = store.bucket().join("collaboration").join(handle.id());
        let names = match crate::disk::metadata_if_present(&storage) {
            Ok(None) => return None,
            Ok(Some(metadata)) if crate::disk::ordinary_directory(&metadata) => {
                fs::read_dir(&storage).and_then(|entries| {
                    entries
                        .map(|entry| entry.map(|entry| entry.file_name()))
                        .collect::<Result<Vec<_>, _>>()
                })
            }
            Ok(Some(_)) => {
                return Some(format!(
                    "Collaborator storage {} is not an ordinary directory; nothing was reclaimed.",
                    storage.display()
                ));
            }
            Err(error) => Err(error),
        };
        let names = match names {
            Ok(names) => names
                .into_iter()
                .filter_map(|name| name.into_string().ok())
                .collect::<Vec<_>>(),
            Err(error) => {
                return Some(format!(
                    "Could not inspect collaborator storage {}: {error}",
                    storage.display()
                ));
            }
        };
        // A deleted conversation's retained record keeps its copies for recovery.
        if names
            .iter()
            .any(|name| name.starts_with("parent-") && name.ends_with(".json"))
        {
            return None;
        }
        let mut removed = 0;
        let mut notes = Vec::new();
        for name in names {
            let Some(number) = name
                .strip_prefix('a')
                .and_then(|number| number.parse::<usize>().ok())
                .filter(|number| format!("a{number}") == name)
            else {
                continue;
            };
            if owned.contains(&name) {
                continue;
            }
            match Workspace::reclaim(store.directory(), &storage, &name) {
                Ok(()) => removed += 1,
                Err(error) => {
                    // New assignments must not collide with a copy left in place.
                    let mut state = self.0.state.lock().unwrap();
                    state.last_number = state.last_number.max(number);
                    notes.push(format!(
                        "Kept isolated agent copy {} without a record: {error}.",
                        storage.join(&name).display()
                    ));
                }
            }
        }
        if removed > 0 {
            notes.insert(
                0,
                format!(
                    "Removed {removed} isolated agent {} that an interrupted setup left without a record.",
                    if removed == 1 { "copy" } else { "copies" }
                ),
            );
        }
        (!notes.is_empty()).then(|| notes.join(" "))
    }
}
