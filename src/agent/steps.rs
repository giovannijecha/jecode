//! One call that runs several tools in order. Models tend to send one call per
//! response, and every response sends the conversation again. The transcript
//! keeps the model's own call with one combined result, because native output
//! is replayed verbatim; each step still has its permission check, checkpoint
//! and card, like a call of its own.

use super::Agent;
use crate::{
    backend::ToolCall,
    events::EventSink,
    file_change::Change,
    json::{self, Value},
    sessions::Stage,
};

/// The running call: step results in order, and its file changes, which the
/// session ties to the one tool message recorded when the call ends.
pub(super) struct Chain {
    pub(super) id: String,
    /// Each step's result with its tool's name.
    pub(super) results: Vec<(String, Value)>,
    /// Each change with the step that made it.
    pub(super) changes: Vec<(String, Change)>,
}

impl Chain {
    /// A plan set earlier in this call, which history does not hold yet.
    pub(super) fn plan(&self) -> Option<crate::todo::Plan> {
        self.results
            .iter()
            .rev()
            .filter(|(tool, _)| tool == "todo_list")
            .find_map(|(_, result)| crate::todo::plan_in(result))
    }
}

impl Agent {
    pub(super) fn execute_steps(
        &mut self,
        call: &ToolCall,
        remaining: &[ToolCall],
        events: &mut impl EventSink,
    ) -> Result<(), String> {
        let steps = match self.expand_steps(call) {
            Ok(steps) => steps,
            Err(error) => return self.refuse_steps(call, &error, remaining, events),
        };
        self.chain = Some(Chain {
            id: call.id.clone(),
            results: Vec::new(),
            changes: Vec::new(),
        });
        let mut outcome = Ok(());
        for step in &steps {
            if self.cancellation.requested() {
                outcome = Err("Operation cancelled".to_string());
                break;
            }
            outcome = self.execute_one(step, &[], events);
            let stopped = self
                .chain
                .as_ref()
                .and_then(|chain| chain.results.last())
                .is_some_and(|(tool, result)| failed(tool, result));
            if outcome.is_err() || stopped {
                break;
            }
        }
        let Chain {
            results, changes, ..
        } = self.chain.take().expect("the chain was installed above");
        self.tool_result(call, &combined(&steps, &results));
        for (step, change) in &changes {
            self.record_tool_change(call, Some(step), change);
        }
        let saved = self.checkpoint(Stage::ToolFinished);
        if let Err(error) = outcome.and(saved) {
            self.cancel_calls(remaining);
            return Err(error);
        }
        Ok(())
    }

    fn expand_steps(&self, call: &ToolCall) -> Result<Vec<ToolCall>, String> {
        let arguments = json::parse(&call.arguments)?;
        let steps = arguments
            .get("steps")
            .and_then(Value::as_array)
            .ok_or("steps must be an array")?;
        if steps.is_empty() {
            return Err("steps must contain at least one entry".into());
        }
        steps
            .iter()
            .enumerate()
            .map(|(index, step)| {
                let tool = step
                    .get("tool")
                    .and_then(Value::as_str)
                    .ok_or("Each step needs a tool name")?;
                // Backends that namespace tools show them to the model as jecode.read.
                let tool = tool.strip_prefix("jecode.").unwrap_or(tool);
                if matches!(tool, "steps" | "ask_user") {
                    return Err(format!("{tool} cannot run as a step"));
                }
                if !self.offered(tool) {
                    return Err(format!("Unknown tool: {tool}"));
                }
                let arguments = match step.get("arguments") {
                    None => "{}".to_string(),
                    Some(Value::String(text)) => json::parse(text)?.encode(),
                    Some(value @ Value::Object(_)) => value.encode(),
                    Some(_) => return Err("Step arguments must be an object".into()),
                };
                // Nested JSON text is decoded as it is for a call of its own.
                let arguments =
                    crate::tools::coerce::arguments(tool, &arguments).unwrap_or(arguments);
                Ok(ToolCall {
                    id: format!("{}_s{}", call.id, index + 1),
                    name: tool.into(),
                    arguments,
                })
            })
            .collect()
    }

    /// Only tools this agent is offered can run as steps.
    fn offered(&self, tool: &str) -> bool {
        crate::tools::definitions_for(!self.child)
            .as_array()
            .is_some_and(|tools| {
                tools
                    .iter()
                    .any(|definition| definition.get("name").and_then(Value::as_str) == Some(tool))
            })
    }

    fn refuse_steps(
        &mut self,
        call: &ToolCall,
        error: &str,
        remaining: &[ToolCall],
        events: &mut impl EventSink,
    ) -> Result<(), String> {
        let result = Value::object([("error", Value::string(error))]);
        let shown = events.emit(self.started_event(call));
        self.tool_result(call, &result);
        let shown = shown
            .and_then(|()| self.checkpoint(Stage::ToolFinished))
            .and_then(|()| events.emit(self.finished_event(call, &result)));
        if shown.is_err() {
            self.cancel_calls(remaining);
        }
        shown
    }
}

/// A step that failed stops the rest: they were planned on its success.
/// An outcome other than `completed` means refused, failed or unknown. LSP
/// queries and background commands report failures through their status.
/// A command that yielded before its outcome was known cannot have succeeded
/// yet, unless the step asked for that with its own `wait_ms`.
/// A multi-file edit fails when any file does, and a restore when any path
/// conflicts, fails or is skipped; a multi-file read reports each path and
/// goes on, as it does alone.
pub(crate) fn failed(tool: &str, result: &Value) -> bool {
    result.get("error").is_some()
        || result
            .get("outcome")
            .is_some_and(|outcome| outcome.as_str() != Some("completed"))
        || result.get("timed_out") == Some(&Value::Bool(true))
        || result.get("cancelled") == Some(&Value::Bool(true))
        || result.get("yielded") == Some(&Value::Bool(true))
        || result.get("moved_to_background") == Some(&Value::Bool(true))
        || (tool == "lsp"
            && result
                .get("status")
                .and_then(Value::as_str)
                .is_some_and(|status| status != "ready"))
        || (matches!(tool, "bash" | "exec" | "process")
            && result.get("status").and_then(Value::as_str) == Some("timed_out"))
        // Stopping is what a process step can ask for; a command did not.
        || (matches!(tool, "bash" | "exec")
            && result.get("status").and_then(Value::as_str) == Some("stopped"))
        || matches!(result.get("exit_code"), Some(Value::Number(code)) if code != "0")
        || (tool == "edit"
            && result
                .get("files")
                .and_then(Value::as_array)
                .is_some_and(|files| files.iter().any(|file| file.get("error").is_some())))
        || (tool == "checkpoint"
            && ["conflict_count", "failed_count", "skipped_count"]
                .iter()
                .any(|count| {
                    result
                        .get(count)
                        .and_then(Value::as_usize)
                        .is_some_and(|count| count > 0)
                }))
}

fn combined(steps: &[ToolCall], results: &[(String, Value)]) -> Value {
    let entries = steps
        .iter()
        .enumerate()
        .map(|(index, step)| {
            let tool = ("tool", Value::string(&step.name));
            match results.get(index) {
                Some((_, result)) => Value::object([
                    tool,
                    ("result", super::read_paths::shown_alone(result.clone())),
                ]),
                None => Value::object([tool, ("outcome", Value::string("not_run"))]),
            }
        })
        .collect();
    Value::object([("steps", Value::Array(entries))])
}

#[cfg(test)]
mod tests;
