use super::Agent;
use crate::{
    backend::ToolCall,
    events::{Event, EventSink, tool_summary},
    json::{self, Value},
    sessions::Stage,
    tools::{Begun, Execution},
};

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
            let end = calls[index..]
                .iter()
                .take(MAX_PARALLEL_CALLS)
                .take_while(|call| parallel_call(call))
                .count()
                + index;
            if end - index >= 2 {
                self.execute_parallel_batch(&calls[index..end], &calls[end..], events)?;
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
        let outcome = self.execute_tool_with_change(call, begun);
        let result = &outcome.result;
        self.tool_result(call, result);
        let change = outcome.change.map(|change| change.redacted(&self.redactor));
        if let Some(change) = &change {
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
        let results = run_parallel_calls(batch, &|call| {
            self.tools.execute_begun(
                Begun::default(),
                &call.name,
                &call.arguments,
                &self.cancellation,
            )
        });
        for (call, outcome) in batch.iter().zip(&results) {
            self.tool_result(call, &outcome.result);
        }
        if let Err(error) = self.checkpoint(Stage::ReadBatchFinished) {
            self.cancel_calls(remaining);
            return Err(error);
        }
        for (call, outcome) in batch.iter().zip(&results) {
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

    fn finished_event(&self, call: &ToolCall, result: &Value) -> Event {
        Event::ToolFinished {
            id: self.redact(&call.id),
            name: self.redact(&call.name),
            summary: self.redact(&tool_summary(&call.name, result)),
            result: self.redactor.value(result),
        }
    }
}

fn parallel_call(call: &ToolCall) -> bool {
    let Ok(arguments) = json::parse(&call.arguments) else {
        return false;
    };
    let action = arguments.get("action").and_then(Value::as_str);
    match call.name.as_str() {
        // History depends on preceding results; skills keep a per-turn snapshot.
        "read" => arguments
            .get("path")
            .and_then(Value::as_str)
            .is_some_and(|path| !path.starts_with("history:") && !path.starts_with("skill:")),
        "checkpoint" => matches!(action, Some("list" | "inspect")),
        "process" => action == Some("list"),
        "web" => {
            let method = arguments
                .get("method")
                .and_then(Value::as_str)
                .unwrap_or("GET");
            matches!(action, Some("fetch" | "download"))
                && matches!(method, "GET" | "HEAD")
                && arguments.get("body").is_none()
        }
        _ => false,
    }
}

fn run_parallel_calls(
    batch: &[ToolCall],
    execute: &(impl Fn(&ToolCall) -> Execution + Sync),
) -> Vec<Execution> {
    std::thread::scope(|scope| {
        let handles = batch
            .iter()
            .map(|call| scope.spawn(|| execute(call)))
            .collect::<Vec<_>>();
        handles
            .into_iter()
            .map(|handle| {
                handle.join().unwrap_or_else(|_| Execution {
                    result: Value::object([(
                        "error",
                        Value::string("Tool worker failed unexpectedly"),
                    )]),
                    change: None,
                })
            })
            .collect()
    })
}

#[cfg(test)]
mod tests;
#[cfg(test)]
mod web_tests;
