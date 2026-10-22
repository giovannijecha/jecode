//! Explicit, bounded protection of named paths before a model runs Bash.
//! No directory is expanded: the caller decides exactly which paths matter.
use super::capture::{self, Observation};
use super::index::reference;
use super::record::{Discovery, Record, State, Version};
use super::scan::in_scope;
use super::session::object;
use super::workspace::Workspace;
use crate::json::Value;
use std::collections::BTreeSet;
use std::sync::Arc;

#[cfg(test)]
mod tests;

const MAX_PATHS: usize = 200;

struct Entry {
    path: String,
    status: &'static str,
    reason: Option<String>,
    state: Option<&'static str>,
    saved_at: Option<u64>,
}

impl Entry {
    fn new(path: String) -> Self {
        Self {
            path,
            status: "pending",
            reason: None,
            state: None,
            saved_at: None,
        }
    }

    fn finish(&mut self, status: &'static str, reason: impl Into<String>) {
        self.status = status;
        self.reason = Some(reason.into());
    }

    fn value(self) -> Value {
        let mut fields = vec![
            ("path", Value::string(self.path)),
            ("status", Value::string(self.status)),
        ];
        if let Some(state) = self.state {
            fields.push(("state", Value::string(state)));
        }
        if let Some(seq) = self.saved_at {
            fields.push(("saved_at", Value::string(reference(seq))));
        }
        if let Some(reason) = self.reason {
            fields.push(("reason", Value::string(reason)));
        }
        object(fields)
    }
}

fn matches_saved(workspace: &Workspace, state: &State, observation: &Observation) -> bool {
    match (state, observation) {
        (State::Absent, Observation::Absent) => true,
        (
            State::File {
                object,
                length,
                digest,
                mode,
            },
            Observation::File(bytes, observed_mode),
        ) => {
            *length == bytes.len() as u64
                && *digest == super::hash::digest(bytes)
                && mode == observed_mode
                && workspace
                    .store()
                    .bytes(*object)
                    .is_ok_and(|old| old[..] == bytes[..])
        }
        (State::Link { target }, Observation::Link(observed)) => target == observed,
        _ => false,
    }
}

/// Capture the current states of explicit file or link names. Every saved_at
/// names a durable version even if a later path or the final point fails.
pub fn run(workspace: &Arc<Workspace>, origin: &str, paths: &[String]) -> Result<Value, String> {
    run_with(workspace, origin, paths, |_| {})
}

fn run_with(
    workspace: &Arc<Workspace>,
    origin: &str,
    paths: &[String],
    mut after_record: impl FnMut(&str),
) -> Result<Value, String> {
    if paths.is_empty() || paths.len() > MAX_PATHS {
        return Err(format!("capture needs 1 to {MAX_PATHS} explicit paths"));
    }
    let (policy, unavailable) = {
        let mut store = workspace.store();
        let error = store.refresh().err();
        workspace.observe_coverage(&store);
        (store.policy.clone(), error)
    };
    let mut seen = BTreeSet::new();
    let mut entries: Vec<Entry> = paths.iter().cloned().map(Entry::new).collect();
    let mut candidates = Vec::new();
    for (index, entry) in entries.iter_mut().enumerate() {
        if !seen.insert(entry.path.clone()) {
            entry.finish("skipped", "duplicate path");
        } else if !in_scope(&policy, &workspace.excluded, &entry.path, false) {
            entry.finish("skipped", "outside the checkpoint scope");
        } else if let Some(error) = &unavailable {
            entry.finish("unprotected", error.clone());
        } else {
            candidates.push(index);
        }
    }
    let names: Vec<&str> = candidates
        .iter()
        .map(|&i| entries[i].path.as_str())
        .collect();
    // One sorted lease acquisition avoids deadlock with overlapping captures.
    let lease = workspace.leases.acquire(&names);
    match lease {
        Err(error) => {
            for index in candidates {
                entries[index].finish(
                    "unprotected",
                    format!("could not coordinate capture: {error}"),
                );
            }
        }
        Ok(_lease) => {
            let mut saved = Vec::new();
            for index in candidates {
                let entry = &mut entries[index];
                let observed =
                    capture::observe(&workspace.root, &entry.path, policy.max_file_bytes);
                match &observed.kind {
                    Observation::Directory(_) => {
                        entry.finish(
                            "skipped",
                            "directory; name the individual file paths to capture",
                        );
                        continue;
                    }
                    Observation::Oversized(length) => {
                        entry.finish(
                            "unprotected",
                            format!("file has {length} bytes and exceeds the capture limit"),
                        );
                        continue;
                    }
                    Observation::Unreadable(reason) => {
                        entry.finish("unprotected", reason.clone());
                        continue;
                    }
                    _ => {}
                }
                let absent = matches!(&observed.kind, Observation::Absent);
                let result = workspace.store().transaction(true, |transaction| {
                    if transaction.store.policy.id() != policy.id() {
                        return Err("checkpoint scope changed during capture; retry".into());
                    }
                    if absent && transaction.index().current(&entry.path).is_none() {
                        // An explicit observation of absence is recoverable. An
                        // unvisited path before this frame remains unknown.
                        let seq = transaction.seq();
                        let discovery = if transaction.index().coverage_complete() {
                            Discovery::Created
                        } else {
                            Discovery::Found
                        };
                        transaction.record(Record::Version(Version {
                            seq,
                            path: entry.path.clone(),
                            state: State::Absent,
                            stat: None,
                            time: crate::sessions::now(),
                            origin: origin.into(),
                            operation: 0,
                            discovery,
                        }));
                    } else {
                        capture::record(transaction, observed, origin, 0)?;
                    }
                    transaction.flush(false)?;
                    let current = transaction
                        .index()
                        .current(&entry.path)
                        .ok_or("capture did not record a path state")?;
                    // Compaction can retain a current version whose original
                    // sequence is below the retention horizon. Name the state
                    // at this transaction's retained frontier instead.
                    Ok((transaction.index().latest(), current.state.clone()))
                });
                match result {
                    Ok((seq, state)) if state.recoverable() => {
                        entry.status = "saved";
                        entry.state = Some(state.label());
                        entry.saved_at = Some(seq);
                        after_record(&entry.path);
                        saved.push((index, state));
                    }
                    Ok((_, state)) => entry.finish(
                        "unprotected",
                        format!("{} cannot be restored", state.label()),
                    ),
                    Err(error) => entry.finish("unprotected", error),
                }
            }
            // A same-size external edit can happen while bytes are copied to
            // the archive. Reread content, not just timestamps, before naming
            // the point. External edits after this check can still race Bash.
            for (index, state) in saved {
                let entry = &mut entries[index];
                let live = capture::observe(&workspace.root, &entry.path, policy.max_file_bytes);
                if !matches_saved(workspace, &state, &live.kind) {
                    entry.finish(
                        "unprotected",
                        "path changed during capture; retry before Bash",
                    );
                    entry.saved_at = None;
                    entry.state = None;
                }
            }
            let count = entries
                .iter()
                .filter(|entry| entry.status == "saved")
                .count();
            let point = (count > 0).then(|| {
                workspace.store().transaction(true, |transaction| {
                    if transaction.store.policy.id() != policy.id() {
                        return Err("checkpoint scope changed during capture; retry".into());
                    }
                    Ok(transaction.point(
                        "capture",
                        &format!("{count} explicit paths"),
                        origin,
                        false,
                    ))
                })
            });
            return Ok(result(entries, point));
        }
    }
    Ok(result(entries, None))
}

fn result(entries: Vec<Entry>, point: Option<Result<u64, String>>) -> Value {
    let saved = entries
        .iter()
        .filter(|entry| entry.status == "saved")
        .count();
    let skipped = entries
        .iter()
        .filter(|entry| entry.status == "skipped")
        .count();
    let unprotected = entries.len() - saved - skipped;
    let mut fields = vec![
        (
            "paths",
            Value::Array(entries.into_iter().map(Entry::value).collect()),
        ),
        ("saved", Value::number(saved)),
        ("skipped", Value::number(skipped)),
        ("unprotected", Value::number(unprotected)),
    ];
    match point {
        Some(Ok(seq)) => fields.push(("ref", Value::string(reference(seq)))),
        Some(Err(error)) => fields.push(("point_error", Value::string(error))),
        None => {}
    }
    object(fields)
}
