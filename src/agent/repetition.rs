use super::Agent;
use crate::json::{self, Value};
use std::collections::BTreeSet;

impl Agent {
    pub(super) fn repetition_refusal(&self, arguments: &Value) -> Option<Value> {
        if let Some(reason) = arguments.get("repeat_reason") {
            match reason.as_str() {
                Some(reason) if !reason.trim().is_empty() => return None,
                // An unused optional field supplies no permission to repeat.
                Some(_) => {}
                None => {
                    return Some(Value::object([
                        (
                            "error",
                            Value::string(
                                "repeat_reason must be a string; a deliberate repeat requires a nonempty explanation",
                            ),
                        ),
                        ("outcome", Value::string("not_started")),
                    ]));
                }
            }
        }
        let command = arguments.get("command")?.as_str()?;
        let messages = self.messages.lock().unwrap();
        let mut calls = BTreeSet::new();
        let mut previous = None;
        for (at, message) in messages.iter().enumerate().take(self.context.from) {
            if message.get("role").and_then(Value::as_str) == Some("assistant") {
                for call in message
                    .get("tool_calls")
                    .and_then(Value::as_array)
                    .unwrap_or(&[])
                {
                    let function = call.get("function");
                    if function
                        .and_then(|function| function.get("name"))
                        .and_then(Value::as_str)
                        == Some("bash")
                        && function
                            .and_then(|function| function.get("arguments"))
                            .and_then(Value::as_str)
                            .and_then(|text| json::parse(text).ok())
                            .and_then(|arguments| {
                                arguments
                                    .get("command")
                                    .and_then(Value::as_str)
                                    .map(str::to_owned)
                            })
                            .as_deref()
                            == Some(command)
                        && let Some(id) = call.get("id").and_then(Value::as_str)
                    {
                        calls.insert(id);
                    }
                }
            } else if message.get("role").and_then(Value::as_str) == Some("tool")
                && message
                    .get("tool_call_id")
                    .and_then(Value::as_str)
                    .is_some_and(|id| calls.remove(id))
                && let Some(result) = message
                    .get("content")
                    .and_then(Value::as_str)
                    .and_then(|text| json::parse(text).ok())
                && result.get("outcome").and_then(Value::as_str) != Some("not_started")
                && (result.get("exit_code").is_some()
                    || result.get("outcome").and_then(Value::as_str) == Some("unknown"))
            {
                previous = Some((at, result));
            }
        }
        previous.map(|(at, result)| Value::object([
            ("error", Value::string("Command not executed: this exact Bash command already has a result outside the active transcript. Review its original result and the user request before repeating it. Continue the next unfinished step when it already ran; never repeat an explicitly once-only operation. If re-execution is actually required (for example a check after new edits), supply repeat_reason explaining the need. An earlier unknown outcome requires investigation; this is not a cached fresh check.")),
            ("outcome", Value::string("not_started")),
            ("previous_history_reference", Value::string(format!("history:{at}"))),
            ("previous_exit_code", result.get("exit_code").cloned().unwrap_or(Value::Null)),
            ("previous_check_status", result.get("check_status").cloned().unwrap_or(Value::Null)),
        ]))
    }
}

#[cfg(test)]
mod tests;
