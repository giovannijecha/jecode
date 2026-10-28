use super::Agent;
use super::timing::Clock;
use crate::{
    backend::ToolCall,
    events::{Event, EventSink, tool_summary},
    json::{self, Value},
    sessions::Stage,
    tools::{Begun, Execution},
};
use std::time::Instant;

mod declared;
mod scheduling;
use scheduling::{Mode, mode};

#[cfg(test)]
use scheduling::parallel_call;

const MAX_PARALLEL_CALLS: usize = 4;

impl Agent {
    pub(super) fn execute_calls(
        &mut self,
        calls: &[ToolCall],
        events: &mut impl EventSink,
    ) -> Result<(), String> {
        let mut index = 0;
        while index < calls.len() {
            if self.cancellation.requested() {
                self.cancel_calls(&calls[index..]);
                return Err("Operation cancelled".into());
            }
            let selected = mode(&calls[index]);
            let end = if selected == Mode::Serial {
                index + 1
            } else {
                index
                    + calls[index..]
                        .iter()
                        .take(MAX_PARALLEL_CALLS)
                        .take_while(|call| mode(call) == selected)
                        .count()
            };
            if end - index >= 2 {
                if selected == Mode::Declared {
                    self.execute_declared_batch(&calls[index..end], &calls[end..], events)?;
                } else {
                    self.execute_parallel_batch(&calls[index..end], &calls[end..], events)?;
                }
                index = end;
            } else {
                self.execute_one(&calls[index], &calls[index + 1..], events)?;
                index += 1;
            }
        }
        Ok(())
    }

    fn execute_one(
        &mut self,
        call: &ToolCall,
        remaining: &[ToolCall],
        events: &mut impl EventSink,
    ) -> Result<(), String> {
        let clock = Clock::new();
        if let Err(error) = self.checkpoint(Stage::Tool(call.id.clone())) {
            self.cancel_calls(std::slice::from_ref(call));
            self.cancel_calls(remaining);
            return Err(error);
        }
        if let Err(error) = events.emit(self.started_event(call)) {
            self.cancel_calls(std::slice::from_ref(call));
            self.cancel_calls(remaining);
            return Err(error);
        }
        let begun = self
            .tools
            .begin_cancellable(&call.name, &call.arguments, &self.cancellation);
        if let Some(note) = begun.note()
            && let Err(error) = events.emit(Event::ToolNote {
                id: self.redact(&call.id),
                text: self.redact(&note),
            })
        {
            self.cancel_calls(std::slice::from_ref(call));
            self.cancel_calls(remaining);
            return Err(error);
        }
        let executing = Instant::now();
        let outcome = self.execute_tool_with_change(call, begun);
        let timing = clock.finish(executing);
        let result = &outcome.result;
        self.tool_result(call, result);
        self.record_tool_timing(call, timing);
        let change = outcome.change.map(|change| change.redacted(&self.redactor));
        if let Some(change) = &change {
            self.record_tool_change(call, change);
        }
        if let Err(error) = self.checkpoint(Stage::ToolFinished) {
            self.cancel_calls(remaining);
            return Err(error);
        }
        if let Some(change) = change
            && let Err(error) = events.emit(Event::ToolChanged {
                id: self.redact(&call.id),
                change,
            })
        {
            self.cancel_calls(remaining);
            return Err(error);
        }
        if let Err(error) = events
            .emit(self.finished_event(call, result))
            .and_then(|()| self.emit_context(events))
        {
            self.cancel_calls(remaining);
            return Err(error);
        }
        Ok(())
    }

    fn execute_parallel_batch(
        &mut self,
        batch: &[ToolCall],
        remaining: &[ToolCall],
        events: &mut impl EventSink,
    ) -> Result<(), String> {
        let clock = Clock::new();
        let ids = batch.iter().map(|call| call.id.clone()).collect();
        if let Err(error) = self.checkpoint(Stage::ReadBatch(ids)) {
            self.cancel_calls(batch);
            self.cancel_calls(remaining);
            return Err(error);
        }
        for call in batch {
            if let Err(error) = events.emit(self.started_event(call)) {
                self.cancel_calls(batch);
                self.cancel_calls(remaining);
                return Err(error);
            }
        }
        // Scoped workers all finish before results or a later, potentially
        // modifying call is handled. Session messages remain in source order.
        let mut results = run_parallel_calls(batch, clock, &|call| {
            self.tools.execute_begun(
                Begun::default(),
                &call.name,
                &call.arguments,
                &self.cancellation,
            )
        });
        for (call, (outcome, timing)) in batch.iter().zip(&mut results) {
            self.tool_result(call, &outcome.result);
            if let Some(timing) = timing.take() {
                self.record_tool_timing(call, timing);
            }
        }
        if let Err(error) = self.checkpoint(Stage::ReadBatchFinished) {
            self.cancel_calls(remaining);
            return Err(error);
        }
        for (call, (outcome, _)) in batch.iter().zip(&results) {
            if let Err(error) = events.emit(self.finished_event(call, &outcome.result)) {
                self.cancel_calls(remaining);
                return Err(error);
            }
        }
        // Every result was recorded together; one context update describes the
        // whole batch rather than recalculating the same projection per result.
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

    fn started_event(&self, call: &ToolCall) -> Event {
        let arguments =
            json::parse(&call.arguments).unwrap_or_else(|_| Value::string(&call.arguments));
        Event::ToolStarted {
            id: self.redact(&call.id),
            name: self.redact(&call.name),
            arguments: self.redactor.value(&arguments),
        }
    }

    fn record_tool_change(&self, call: &ToolCall, change: &crate::file_change::Change) {
        self.events.lock().unwrap().push(Value::object([
            ("type", Value::string("tool_change")),
            (
                "after_message",
                Value::number(self.messages.lock().unwrap().len()),
            ),
            ("tool_call_id", Value::string(self.redact(&call.id))),
            ("change", change.value()),
        ]));
    }

    fn finished_event(&self, call: &ToolCall, result: &Value) -> Event {
        Event::ToolFinished {
            id: self.redact(&call.id),
            name: self.redact(&call.name),
            summary: self.redact(&tool_summary(&call.name, result)),
            result: self.redactor.value(result),
        }
    }
}

fn run_parallel_calls(
    batch: &[ToolCall],
    clock: Clock,
    execute: &(impl Fn(&ToolCall) -> Execution + Sync),
) -> Vec<(Execution, Option<Value>)> {
    std::thread::scope(|scope| {
        let handles = batch
            .iter()
            .map(|call| {
                scope.spawn(move || {
                    let executing = Instant::now();
                    let outcome = execute(call);
                    (outcome, Some(clock.finish(executing)))
                })
            })
            .collect::<Vec<_>>();
        handles
            .into_iter()
            .map(|handle| {
                handle.join().unwrap_or_else(|_| {
                    (
                        Execution {
                            result: Value::object([(
                                "error",
                                Value::string("Tool worker failed unexpectedly"),
                            )]),
                            change: None,
                        },
                        None,
                    )
                })
            })
            .collect()
    })
}

#[cfg(test)]
mod provider_tests;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod web_tests;
