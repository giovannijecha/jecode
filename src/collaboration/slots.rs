//! A conversation starts at most eight collaborator turns concurrently. Queue
//! order is fixed when an assignment is accepted, before any thread is started.

use super::{Observer, State};
use crate::agent::Agent;
use std::{
    panic::AssertUnwindSafe,
    sync::{Arc, Mutex},
    thread,
};

pub(super) const RUNNING: usize = 8;

/// The caller holds the state lock. Cancelled assignments leave the queue
/// without consuming a slot or starting a model request.
fn take_next(
    state: &mut State,
    cancelled: &mut Vec<String>,
) -> Option<(String, Arc<Mutex<Agent>>)> {
    while state.running < RUNNING {
        let id = state.queue.front()?.clone();
        let Some(index) = state.workers.iter().position(|worker| worker.id == id) else {
            state.queue.pop_front();
            continue;
        };
        if !state.workers[index].ready {
            return None;
        }
        state.queue.pop_front();
        if state.workers[index].cancellation.requested() {
            if let Some(record) = state.record(&id)
                && record.state == "running"
            {
                record.state = "stopping";
                cancelled.push(id);
            }
            continue;
        }
        if !state
            .records
            .iter()
            .any(|record| record.id == id && record.state == "running")
        {
            continue;
        }
        let child = Arc::clone(&state.workers[index].child);
        state.workers[index].busy = true;
        state.running += 1;
        if let Some(record) = state.record(&id) {
            record.activity = "Starting".into();
        }
        return Some((id, child));
    }
    None
}

impl Observer {
    #[cfg(test)]
    pub(crate) fn slot_stats(&self) -> (usize, Vec<String>, usize) {
        let state = self.0.state.lock().unwrap();
        (
            state.running,
            state.queue.iter().cloned().collect(),
            state.workers.iter().filter(|worker| worker.busy).count(),
        )
    }

    pub(crate) fn release_created(&self, ids: &[String]) {
        {
            let mut state = self.0.state.lock().unwrap();
            for worker in &mut state.workers {
                if ids.contains(&worker.id) {
                    worker.ready = true;
                }
            }
        }
        self.dispatch();
    }

    /// Start at most eight OS threads. A thread that finishes one turn takes
    /// the next queued turn itself, so fast completions cannot grow the thread
    /// count before their former workers have exited.
    pub(super) fn dispatch(&self) {
        let mut failures = Vec::new();
        let mut cancelled = Vec::new();
        let mut state = self.0.state.lock().unwrap();
        while let Some((id, child)) = take_next(&mut state, &mut cancelled) {
            let observer = Self(Arc::clone(&self.0), None);
            let thread_id = id.clone();
            // A worker goes on to run later queued turns, so its name is generic.
            let start = thread::Builder::new()
                .name("jecode-collaborator".into())
                .spawn(move || observer.run_worker(thread_id, child));
            if let Err(error) = start {
                state.running -= 1;
                if let Some(worker) = state.workers.iter_mut().find(|worker| worker.id == id) {
                    worker.busy = false;
                }
                if let Some(record) = state.record(&id) {
                    record.state = "failed";
                    record.error = Some(format!("Could not start collaborator: {error}"));
                    record.activity = "Failed".into();
                    failures.push((
                        id,
                        record.received_version,
                        record.title.clone(),
                        record.error.clone().unwrap(),
                    ));
                }
            }
        }
        drop(state);
        self.0.changed.notify_all();
        for id in cancelled {
            self.finish_stopped(&id);
        }
        for (id, version, title, error) in failures {
            if let Err(error) = self.receipt("returned", Some(&id), version, &title, vec![error]) {
                self.persistence_failed(&id, error);
            }
        }
    }

    fn run_worker(&self, mut id: String, mut child: Arc<Mutex<Agent>>) {
        loop {
            let ran = std::panic::catch_unwind(AssertUnwindSafe(|| {
                let mut child = child.lock().unwrap_or_else(|error| error.into_inner());
                self.run_child(&id, &mut child);
            }));
            let Some((next_id, next_child)) = self.finish_turn(&id, ran.is_err()) else {
                break;
            };
            id = next_id;
            child = next_child;
        }
    }

    /// Releases a finished turn's slot and takes the next queued turn.
    pub(super) fn finish_turn(
        &self,
        id: &str,
        panicked: bool,
    ) -> Option<(String, Arc<Mutex<Agent>>)> {
        let id = id.to_owned();
        let mut cancelled = Vec::new();
        let mut state = self.0.state.lock().unwrap();
        state.running -= 1;
        if let Some(worker) = state.workers.iter_mut().find(|worker| worker.id == id) {
            worker.busy = false;
        }
        if panicked && let Some(record) = state.record(&id) {
            record.state = "unknown";
            record.error = Some("Collaborator worker panicked".into());
            record.activity = "Outcome could not be confirmed".into();
        }
        // A stop that arrived after the turn settled its outcome still needs
        // its receipt.
        if state
            .records
            .iter()
            .any(|record| record.id == id && record.state == "stopping")
        {
            cancelled.push(id.clone());
        }
        if state.records.iter().any(|record| {
            record.id == id && record.state == "running" && !record.pending.is_empty()
        }) && !state.queue.contains(&id)
        {
            state.queue.push_back(id.clone());
            if let Some(record) = state.record(&id) {
                record.activity = "Waiting for a free slot".into();
            }
        }
        let next = take_next(&mut state, &mut cancelled);
        drop(state);
        self.0.changed.notify_all();
        for cancelled in cancelled {
            self.finish_stopped(&cancelled);
        }
        next
    }
}
