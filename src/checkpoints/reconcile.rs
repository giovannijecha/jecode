//! Classifies operations that processes left prepared but incomplete when
//! they exited, by comparing the path with the recorded before and after.
use super::capture::{self, Observed};
use super::locks::owner_gone;
use super::record::{Event, Outcome, Prepare, Record, State};
use super::store::Store;
use super::workspace::Workspace;

/// Completes the stale operations; returns how many were classified.
pub fn incomplete(workspace: &Workspace) -> Result<usize, String> {
    let stale = {
        let mut store = workspace.store();
        store.refresh()?;
        let directory = store.workspace.directory.clone();
        let own = store.owner.clone();
        let stale: Vec<Prepare> = store
            .index
            .pending
            .values()
            .filter(|prepare| prepare.owner != own && owner_gone(&directory, &prepare.owner))
            .cloned()
            .collect();
        remove_owner_files(&store);
        stale
    };
    if stale.is_empty() {
        return Ok(0);
    }
    let names: Vec<&str> = stale.iter().map(|prepare| prepare.path.as_str()).collect();
    let _lease = workspace
        .leases
        .acquire(&names)
        .map_err(|error| format!("Could not coordinate recovery: {error}"))?;
    let mut store = workspace.store();
    let max = store.policy.max_file_bytes;
    let observed: Vec<Observed> = stale
        .iter()
        .map(|prepare| capture::observe(&workspace.root, &prepare.path, max))
        .collect();
    store.transaction(true, |transaction| {
        let mut classified = 0;
        for (prepare, observed) in stale.iter().zip(observed) {
            // Another process may have classified it meanwhile.
            if !transaction.index().pending.contains_key(&prepare.seq) {
                continue;
            }
            let outcome = if matches_state(transaction.store, &observed, &prepare.after) {
                Outcome::Applied
            } else if matches_state(transaction.store, &observed, &prepare.before) {
                Outcome::NotApplied
            } else {
                Outcome::Unknown
            };
            let operation = if outcome == Outcome::Applied {
                prepare.seq
            } else {
                0
            };
            capture::record(transaction, observed, &prepare.origin, operation)?;
            let seq = transaction.seq();
            transaction.record(Record::Complete {
                seq,
                operation: prepare.seq,
                outcome,
            });
            let seq = transaction.seq();
            transaction.record(Record::Event(Event {
                seq,
                origin: "recovery".into(),
                time: crate::sessions::now(),
                text: format!(
                    "an interrupted {} of {} by {} was found {}; current state is recorded",
                    prepare.kind,
                    prepare.path,
                    prepare.origin,
                    match outcome {
                        Outcome::Applied => "applied",
                        Outcome::NotApplied => "not applied",
                        Outcome::Unknown => "in an unknown state",
                    }
                ),
                paths: vec![prepare.path.clone()],
            }));
            classified += 1;
        }
        Ok(classified)
    })
}

fn matches_state(store: &Store, observed: &Observed, state: &State) -> bool {
    capture::matches_state(store, observed, state, false)
}

/// Owner lock files of exited processes without pending operations.
fn remove_owner_files(store: &Store) {
    let directory = super::layout::owners_directory(&store.workspace.directory);
    let Ok(entries) = std::fs::read_dir(&directory) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let Some(owner) = name.strip_suffix(".lock") else {
            continue;
        };
        // A new owner creates its file just before locking it.
        let recent = entry
            .metadata()
            .and_then(|metadata| metadata.modified())
            .ok()
            .and_then(|modified| modified.elapsed().ok())
            .is_none_or(|age| age < std::time::Duration::from_secs(60));
        if recent
            || owner == store.owner
            || store
                .index
                .pending
                .values()
                .any(|prepare| prepare.owner == owner)
        {
            continue;
        }
        if let Ok(Some(held)) = super::locks::Exclusive::try_acquire(&entry.path()) {
            drop(held);
            let _ = std::fs::remove_file(entry.path());
        }
    }
}
