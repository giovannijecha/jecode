//! One process's waits, and the reports that end them.

use super::paging::{OutputMode, read_output};
use super::{Entry, Outcome, State, WaitEnd};
use crate::cancel::Cancellation;
use crate::json::Value;
use crate::process::RunError;
use std::collections::BTreeMap;
use std::time::{Duration, Instant};

/// How long a tree may keep running after its root exits before a waiting
/// call reports that instead. Ordinary commands finish within it, including
/// helpers such as a console host that end right after the root.
pub(super) const ROOT_SETTLE: Duration = Duration::from_millis(500);

impl Entry {
    /// Waits as a new caller. `ignore_root` keeps waiting for the whole tree;
    /// otherwise a root exit first seen during this wait ends it.
    pub(super) fn wait(
        &self,
        wait: Duration,
        cancellation: &Cancellation,
        ignore_root: bool,
    ) -> WaitEnd {
        let (released, root_known) = self.begin_wait();
        self.wait_held(
            wait,
            cancellation,
            Some(released),
            ignore_root || root_known,
        )
    }

    /// Register the waiter and its release generation as one state change.
    pub(super) fn begin_wait(&self) -> (usize, bool) {
        let mut state = self.state.lock().unwrap();
        state.waiters += 1;
        (state.released, state.root.is_some())
    }

    /// Waits as a call already counted in `waiters`, which stays counted until
    /// it reports, so a process ending in between still belongs to that report.
    /// Unless `root_known`, the wait also ends once the root process has been
    /// seen to exit and the tree is still running after `ROOT_SETTLE`.
    pub(super) fn wait_held(
        &self,
        wait: Duration,
        cancellation: &Cancellation,
        released: Option<usize>,
        root_known: bool,
    ) -> WaitEnd {
        // An unrepresentable deadline waits until completion or cancellation.
        let deadline = Instant::now().checked_add(wait);
        let mut state = self.state.lock().unwrap();
        while state.outcome.is_none() {
            if cancellation.requested() {
                return WaitEnd::Cancelled;
            }
            if released.is_some_and(|released| state.released != released) {
                return WaitEnd::Released;
            }
            let mut slice = deadline.map_or(Duration::from_millis(20), |deadline| {
                deadline
                    .saturating_duration_since(Instant::now())
                    .min(Duration::from_millis(20))
            });
            if !root_known && let Some(seen) = state.root_seen {
                let settle = ROOT_SETTLE.saturating_sub(seen.elapsed());
                if settle.is_zero() {
                    return WaitEnd::RootExited;
                }
                slice = slice.min(settle);
            }
            if slice.is_zero() {
                break;
            }
            state = self.changed.wait_timeout(state, slice).unwrap().0;
        }
        WaitEnd::Elapsed
    }

    /// Copies the latest root report into the state and wakes waiters. A
    /// finished process keeps the facts its outcome recorded.
    /// Returns true only for the first root report, to schedule one settle check.
    pub(super) fn observe_root(&self) -> bool {
        let root = self.watch.root();
        let mut first = false;
        self.transition(|state| {
            if state.outcome.is_none() && root.is_some() {
                first = state.root_seen.is_none();
                state.root_seen.get_or_insert_with(Instant::now);
                state.root = root;
            }
        });
        self.changed.notify_all();
        self.notify();
        first
    }

    /// A root-only notice is due only after the ordinary completion grace and
    /// only when no waiting call is about to report the same fact.
    pub(super) fn settle_root(&self) {
        let Some(scope) = self.scope.upgrade() else {
            return;
        };
        let _entries = scope.entries.lock().unwrap();
        let mut state = self.state.lock().unwrap();
        if state.outcome.is_some()
            || !state.returned
            || state.waiters > 0
            || state.root_reported
            || state.root_notice.is_some()
        {
            return;
        }
        let Some(root) = state.root.clone() else {
            return;
        };
        let sequence = scope
            .sequence
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
            + 1;
        state.root_notice = Some((sequence, root));
        drop(state);
        drop(_entries);
        self.notify();
    }

    /// Asks a running process to stop. One that already ended keeps its
    /// outcome, and remains a process that ended by itself.
    pub(super) fn request_stop(&self) {
        let state = self.state.lock().unwrap();
        if state.outcome.is_none() {
            self.stop.cancel();
        }
    }

    /// The final state for a foreground call, which reports it itself.
    pub(super) fn final_outcome(&self) -> Option<Outcome> {
        let mut state = self.state.lock().unwrap();
        let outcome = state.outcome.clone()?;
        state.waiters -= 1;
        state.reported = true;
        Some(outcome)
    }

    pub(super) fn status_name(&self, state: &State) -> &'static str {
        match &state.outcome {
            Some(outcome) => outcome.status,
            None if self.stop.requested() => "stopping",
            None => "running",
        }
    }

    pub(super) fn elapsed(&self, state: &State) -> Duration {
        state
            .finished
            .map_or_else(|| self.started.elapsed(), |end| end - self.started)
    }

    pub(super) fn status(&self, state: &State) -> BTreeMap<String, Value> {
        let mut fields = BTreeMap::new();
        fields.insert("process_id".into(), Value::string(&self.id));
        fields.insert("status".into(), Value::string(self.status_name(state)));
        if let Some(terminal) = &self.terminal {
            let (cols, rows) = terminal.size();
            fields.insert("tty".into(), Value::Bool(true));
            fields.insert("cols".into(), Value::number(cols));
            fields.insert("rows".into(), Value::number(rows));
        }
        if let Some(outcome) = &state.outcome {
            if outcome.exit_code.is_some() || outcome.status == "exited" {
                fields.insert(
                    "exit_code".into(),
                    outcome.exit_code.map_or(Value::Null, Value::number),
                );
            }
            if let Some(error) = &outcome.error {
                fields.insert("error".into(), Value::string(error));
                fields.insert("outcome".into(), Value::string("unknown"));
            }
            // An exited process's code is its root's; otherwise the earlier
            // root exit explains what the timeout or stop ended.
            if outcome.status != "exited"
                && let Some(root) = &outcome.root
            {
                fields.extend(super::root::fields(root, false));
            }
        } else if let Some(root) = &state.root {
            fields.extend(super::root::fields(root, true));
        }
        fields
    }

    /// Ends one call's wait with state, then output after each stream cursor.
    /// State is read first: a final state guarantees that the saved streams
    /// are complete. A final state reported here needs no later announcement.
    pub(super) fn report(
        &self,
        end: WaitEnd,
        output_limit: usize,
        output_mode: OutputMode,
    ) -> Result<Value, String> {
        let mut state = self.state.lock().unwrap();
        state.waiters -= 1;
        state.returned = true;
        let mut fields = self.status(&state);
        let finished = state.outcome.is_some();
        let mut cursors = state.cursors;
        for (index, name) in ["stdout", "stderr"].into_iter().enumerate() {
            let stream = &self.streams[index];
            let from = state.cursors[index];
            let chunk = match read_output(&stream.path, from, finished, output_limit, output_mode) {
                Ok(chunk) => chunk,
                Err(error) => {
                    // This call cannot carry the outcome, so it is still owed.
                    drop(state);
                    self.transition(|_| {});
                    // A failed output read cannot carry an earlier root exit
                    // either. The settle timer may have found this waiter
                    // still active, so publish the now unreported root.
                    self.settle_root();
                    return Err(format!(
                        "Could not read saved {name} for {}: {error}",
                        self.id
                    ));
                }
            };
            cursors[index] = chunk.next;
            fields.insert(name.into(), Value::string(chunk.text));
            fields.insert(format!("{name}_offset"), Value::number(chunk.offset));
            fields.insert(format!("{name}_next_offset"), Value::number(chunk.next));
            fields.insert(
                format!("{name}_remaining_bytes"),
                Value::number(chunk.length - chunk.next),
            );
            fields.insert(
                format!("{name}_skipped_bytes"),
                Value::number(chunk.offset - from),
            );
            fields.insert(format!("{name}_file"), Value::string(&stream.reference));
        }
        state.cursors = cursors;
        state.reported |= finished;
        state.root_reported |= finished || fields.contains_key("root_process");
        fields.insert("output_limit_bytes".into(), Value::number(output_limit));
        fields.insert("output_mode".into(), Value::string(output_mode.name()));
        match end {
            WaitEnd::Elapsed | WaitEnd::RootExited => {}
            WaitEnd::Cancelled => {
                fields.insert("wait_cancelled".into(), Value::Bool(true));
            }
            WaitEnd::Released => {
                fields.insert("moved_to_background".into(), Value::Bool(true));
            }
        }
        Ok(Value::Object(fields))
    }
}

impl From<Result<crate::process::Output, RunError>> for Outcome {
    fn from(result: Result<crate::process::Output, RunError>) -> Self {
        let (stdout_bytes, stderr_bytes) = result
            .as_ref()
            .map_or((0, 0), |output| (output.stdout_bytes, output.stderr_bytes));
        let supervised = result.is_ok();
        let (status, exit_code, error) = match result {
            Ok(output) if output.cancelled => ("stopped", None, None),
            Ok(output) if output.timed_out => ("timed_out", None, None),
            Ok(output) if output.input_error.is_some() => (
                "unknown",
                output.exit_code,
                Some(format!(
                    "The command started but input delivery failed: {}. Execution may already have changed the workspace; inspect it before repeating the command.",
                    output.input_error.unwrap()
                )),
            ),
            Ok(output) => ("exited", output.exit_code, None),
            Err(error) => (
                "unknown",
                None,
                Some(format!(
                    "The command started but its completion or cleanup could not be confirmed: {}. Execution may already have changed the workspace; inspect it before repeating the command.",
                    error.message
                )),
            ),
        };
        Self {
            status,
            exit_code,
            error,
            stdout_bytes,
            stderr_bytes,
            supervised,
            root: None,
        }
    }
}
