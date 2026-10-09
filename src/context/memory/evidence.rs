use crate::json::Value;

/// Derived provenance, never proposed by the model or stored in original messages.
pub fn record(messages: &[Value], index: usize) -> Value {
    let message = messages.get(index);
    let role = message
        .and_then(|message| message.get("role"))
        .and_then(Value::as_str)
        .unwrap_or("missing");
    let call = message
        .filter(|_| role == "tool")
        .and_then(|message| message.get("tool_call_id"))
        .and_then(Value::as_str)
        .and_then(|id| {
            messages
                .iter()
                .enumerate()
                .take(index)
                .rev()
                .find_map(|(at, message)| {
                    if message.get("role").and_then(Value::as_str) != Some("assistant") {
                        return None;
                    }
                    message
                        .get("tool_calls")?
                        .as_array()?
                        .iter()
                        .find_map(|call| {
                            (call.get("id").and_then(Value::as_str) == Some(id))
                                .then_some((at, call))
                        })
                })
        });
    let origin = if role == "tool" {
        call.map(|(at, _)| at)
    } else {
        message.map(|_| index)
    };
    let request = origin.and_then(|at| {
        messages[..=at]
            .iter()
            .rposition(|message| message.get("role").and_then(Value::as_str) == Some("user"))
    });
    let reference = |at| Value::string(format!("history:{at}"));
    Value::object([
        ("history", reference(index)),
        ("role", Value::string(role)),
        ("request_history", request.map_or(Value::Null, reference)),
        (
            "call_history",
            call.map_or(Value::Null, |(at, _)| reference(at)),
        ),
        (
            "tool",
            call.and_then(|(_, call)| call.get("function")?.get("name")?.as_str())
                .map_or(Value::Null, Value::string),
        ),
        (
            "eligible_proof",
            Value::Array(
                kinds(messages, index)
                    .into_iter()
                    .map(Value::string)
                    .collect(),
            ),
        ),
    ])
}

// Use the same validator as memory acceptance; metadata cannot relax its contract.
fn kinds(messages: &[Value], index: usize) -> Vec<&'static str> {
    ["decision", "inspection", "change", "check"]
        .into_iter()
        .filter(|kind| {
            let proposal = Value::object([(
                "completed",
                Value::Array(vec![Value::object([
                    ("description", Value::string("Reference eligibility")),
                    ("kind", Value::string(*kind)),
                    (
                        "evidence",
                        Value::Array(vec![Value::string(format!("history:{index}"))]),
                    ),
                ])]),
            )]);
            super::entries(&proposal, "completed", messages).is_ok()
        })
        .collect()
}

#[cfg(test)]
mod tests;
