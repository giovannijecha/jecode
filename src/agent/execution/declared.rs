//! Explicit independent operations: durable completion order, canonical history order.
use super::{
    Agent, Begun, Clock, Event, EventSink, Execution, Stage, ToolCall, Value, tool_summary,
};
use std::{
    panic::{AssertUnwindSafe, catch_unwind},
    sync::mpsc,
    time::Instant,
};

impl Agent {
    pub(super) fn execute_declared_batch(
        &mut self,
        batch: &[ToolCall],
        remaining: &[ToolCall],
        events: &mut impl EventSink,
    ) -> Result<(), String> {
        let clock = Clock::new();
        let ids = batch.iter().map(|call| call.id.clone()).collect();
        if let Err(error) = self.checkpoint(Stage::ToolBatch(ids)) {
            return self.abort_declared(batch, remaining, error);
        }
        for call in batch {
            if let Err(error) = events.emit(self.started_event(call)) {
                return self.abort_declared(batch, remaining, error);
            }
        }
        let mut prepared = Vec::with_capacity(batch.len());
        for call in batch {
            if self.cancellation.requested() {
                return self.abort_declared(batch, remaining, "Operation cancelled".into());
            }
            let begun =
                self.tools
                    .begin_cancellable(&call.name, &call.arguments, &self.cancellation);
            if let Some(note) = begun.note()
                && let Err(error) = events.emit(Event::ToolNote {
                    id: self.redact(&call.id),
                    text: self.redact(&note),
                })
            {
                return self.abort_declared(batch, remaining, error);
            }
            prepared.push(begun);
        }
        if self.cancellation.requested() {
            return self.abort_declared(batch, remaining, "Operation cancelled".into());
        }

        // Workers borrow Tools, while the controller saves completed receipts.
        // Snapshot immutable request metadata rather than borrowing all of Agent.
        let save = {
            let handle = self.persistence.clone();
            let archive = self.archive();
            let compatible = self.compatible_from;
            let context = self.context.clone();
            let routing = self.routing().clone();
            let has_model = self.has_model();
            move |stage| match &handle {
                Some(handle) => {
                    handle.checkpoint(&archive, compatible, &context, &routing, has_model, stage)
                }
                None => Ok(()),
            }
        };
        let redactor = self.redactor.clone();
        let cancellation = self.cancellation.clone();
        let mut failure = None;
        let mut outcomes = (0..batch.len()).map(|_| None).collect::<Vec<_>>();
        let tools = &self.tools;
        run_workers(
            batch,
            prepared,
            clock,
            &|call, begun| tools.execute_begun(begun, &call.name, &call.arguments, &cancellation),
            |index, outcome, timing| {
                let call = &batch[index];
                let outcome = Execution {
                    result: redactor.value(&outcome.result),
                    change: outcome.change.map(|change| change.redacted(&redactor)),
                };
                if failure.is_none() {
                    let receipt = Value::object([
                        ("id", Value::string(redactor.text(&call.id))),
                        ("result", outcome.result.clone()),
                        ("timing", timing.clone()),
                        (
                            "change",
                            outcome
                                .change
                                .as_ref()
                                .map_or(Value::Null, |change| change.value()),
                        ),
                    ]);
                    let publish = || -> Result<(), String> {
                        save(Stage::ToolBatchResult(receipt))?;
                        if let Some(change) = &outcome.change {
                            events.emit(Event::ToolChanged {
                                id: redactor.text(&call.id),
                                change: change.clone(),
                            })?;
                        }
                        events.emit(Event::ToolFinished {
                            id: redactor.text(&call.id),
                            name: redactor.text(&call.name),
                            summary: redactor.text(&tool_summary(&call.name, &outcome.result)),
                            result: outcome.result.clone(),
                        })
                    };
                    if let Err(error) = publish() {
                        failure = Some(error);
                        cancellation.cancel();
                    }
                }
                outcomes[index] = Some((outcome, timing));
            },
        );

        // Even after cancellation or a display/save failure, every launched
        // worker is joined and every returned outcome is kept, without replay.
        for (call, outcome) in batch.iter().zip(outcomes) {
            let (outcome, timing) = outcome.unwrap_or_else(|| (unknown(), Value::Null));
            self.tool_result(call, &outcome.result);
            if !matches!(timing, Value::Null) {
                self.record_tool_timing(call, timing);
            }
            if let Some(change) = outcome.change {
                self.record_tool_change(call, &change);
            }
        }
        if let Err(error) = self.checkpoint(Stage::ToolBatchFinished) {
            failure.get_or_insert(error);
        }
        if let Some(error) = failure {
            self.cancel_calls(remaining);
            return Err(error);
        }
        if let Err(error) = self.emit_context(events) {
            self.cancel_calls(remaining);
            return Err(error);
        }
        if self.cancellation.requested() {
            self.cancel_calls(remaining);
            return Err("Operation cancelled".into());
        }
        Ok(())
    }

    fn abort_declared(
        &mut self,
        batch: &[ToolCall],
        remaining: &[ToolCall],
        error: String,
    ) -> Result<(), String> {
        self.cancel_calls(batch);
        self.cancel_calls(remaining);
        let _ = self.checkpoint(Stage::ToolBatchFinished);
        Err(error)
    }
}

fn unknown() -> Execution {
    Execution {
        result: Value::object([
            (
                "error",
                Value::string(
                    "Tool outcome is unknown: a worker failed unexpectedly. Check current state before deciding whether to run it again.",
                ),
            ),
            ("outcome", Value::string("unknown")),
        ]),
        change: None,
    }
}

fn run_workers(
    batch: &[ToolCall],
    prepared: Vec<Begun>,
    clock: Clock,
    execute: &(impl Fn(&ToolCall, Begun) -> Execution + Sync),
    mut completed: impl FnMut(usize, Execution, Value),
) {
    std::thread::scope(|scope| {
        let (sender, receiver) = mpsc::channel();
        for (index, (call, begun)) in batch.iter().zip(prepared).enumerate() {
            let sender = sender.clone();
            scope.spawn(move || {
                let executing = Instant::now();
                let outcome = catch_unwind(AssertUnwindSafe(|| execute(call, begun)))
                    .unwrap_or_else(|_| unknown());
                let _ = sender.send((index, outcome, clock.finish(executing)));
            });
        }
        drop(sender);
        for (index, outcome, timing) in receiver {
            completed(index, outcome, timing);
        }
    });
}

#[cfg(test)]
mod lifecycle_tests;
#[cfg(test)]
mod tests;
