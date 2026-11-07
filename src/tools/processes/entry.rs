//! One process's waits, and the reports that end them.

use super::paging::{OutputMode, read_output};
use super::{Entry, Outcome, State, WaitEnd};
use crate::cancel::Cancellation;
use crate::json::Value;
use crate::process::RunError;
use std::collections::BTreeMap;
use std::time::{Duration, Instant};

impl Entry {
    pub(super) fn wait(&self, wait: Duration, cancellation: &Cancellation) -> WaitEnd {
        let released = self.begin_wait();
        self.wait_held(wait, cancellation, Some(released))
    }

    /// Register the waiter and its release generation as one state change.
    pub(super) fn begin_wait(&self) -> usize {
        let mut state = self.state.lock().unwrap();
        state.waiters += 1;
        state.released
    }

    /// Waits as a call already counted in `waiters`, which stays counted until
    /// it reports, so a process ending in between still belongs to that report.
    pub(super) fn wait_held(
        &self,
        wait: Duration,
        cancellation: &Cancellation,
        released: Option<usize>,
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
            let slice = deadline.map_or(Duration::from_millis(20), |deadline| {
                deadline
                    .saturating_duration_since(Instant::now())
                    .min(Duration::from_millis(20))
            });
            if slice.is_zero() {
                break;
            }
            state = self.changed.wait_timeout(state, slice).unwrap().0;
        }
        WaitEnd::Elapsed
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
                    self.announce(&mut state);
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
        fields.insert("output_limit_bytes".into(), Value::number(output_limit));
        fields.insert("output_mode".into(), Value::string(output_mode.name()));
        match end {
            WaitEnd::Elapsed => {}
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
        }
    }
}
