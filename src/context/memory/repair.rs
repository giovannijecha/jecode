use crate::json::{self, Value};

pub fn has_update_facts(candidate: &str, limit: usize) -> bool {
    if candidate.len() > limit {
        return false;
    }
    let Ok(mut value) = json::parse(candidate) else {
        return false;
    };
    super::pending::normalize(&mut value).is_ok()
        && ["objective", "next_action"]
            .iter()
            .all(|key| super::nonempty(&value, key).is_ok())
        && ["constraints", "remaining"]
            .iter()
            .all(|key| super::strings(&value, key).is_ok())
        && value.get("completed").and_then(Value::as_array).is_some()
}

/// Repair uses the rejected proposal plus native proof facts, rather than
/// resummarizing code and output payloads that the first proposal already saw.
pub fn sources(messages: &[Value], indices: impl IntoIterator<Item = usize>) -> String {
    let entries = indices
        .into_iter()
        .map(|index| {
            let mut source = super::source_record(messages, index);
            let Some(message) = messages.get(index) else {
                return source;
            };
            let Value::Object(fields) = &mut source else {
                return source;
            };
            match message.get("role").and_then(Value::as_str) {
                Some("user") => {
                    if let Some(content) = message.get("content") {
                        fields.insert("content".into(), content.clone());
                    }
                }
                Some("tool") => {
                    if let Some(result) = message
                        .get("content")
                        .and_then(Value::as_str)
                        .and_then(|text| json::parse(text).ok())
                    {
                        let facts = [
                            "path",
                            "error",
                            "outcome",
                            "exit_code",
                            "cancelled",
                            "timed_out",
                            "check",
                            "check_status",
                            "bytes_written",
                            "file_tracking",
                            "file_changes",
                        ]
                        .into_iter()
                        .filter_map(|key| result.get(key).map(|value| (key.into(), value.clone())))
                        .collect();
                        fields.insert("result_facts".into(), Value::Object(facts));
                    }
                    if let Some(arguments) = call_arguments(messages, index) {
                        fields.insert("call_arguments".into(), arguments);
                    }
                }
                _ => {}
            }
            source
        })
        .collect();
    format!(
        "Native sources for the rejected transcript portion (code and output payloads remain in original history):\n{}\n",
        Value::Array(entries).encode()
    )
}

fn call_arguments(messages: &[Value], index: usize) -> Option<Value> {
    let id = messages.get(index)?.get("tool_call_id")?.as_str()?;
    messages[..index].iter().rev().find_map(|message| {
        if message.get("role").and_then(Value::as_str) != Some("assistant") {
            return None;
        }
        message
            .get("tool_calls")?
            .as_array()?
            .iter()
            .find_map(|call| {
                if call.get("id")?.as_str()? != id {
                    return None;
                }
                let arguments = call.get("function")?.get("arguments")?.as_str()?;
                let value = json::parse(arguments).ok()?;
                Some(Value::Object(
                    [
                        "path",
                        "paths",
                        "command",
                        "check",
                        "action",
                        "reason",
                        "require_check",
                        "scope_exclusions",
                    ]
                    .into_iter()
                    .filter_map(|key| value.get(key).map(|value| (key.into(), value.clone())))
                    .collect(),
                ))
            })
    })
}

#[cfg(test)]
mod tests;
