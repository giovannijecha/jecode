use super::nonempty;
use crate::json::{self, Value};

fn index(reference: &Value) -> Result<usize, String> {
    let text = reference
        .as_str()
        .and_then(|text| text.strip_prefix("history:"))
        .ok_or("Evidence must reference history:N")?;
    if text.is_empty() || !text.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err("Evidence must reference a zero-based history:N message".into());
    }
    text.parse()
        .map_err(|_| "Evidence history index is out of range".into())
}

pub(super) fn reviewed_request(previous: &Value, messages: &[Value]) -> Option<usize> {
    previous
        .get("reviewed_request_history")
        .and_then(Value::as_usize)
        .filter(|at| {
            messages
                .get(*at)
                .and_then(|message| message.get("role"))
                .and_then(Value::as_str)
                == Some("user")
        })
}

pub(super) fn has_new_evidence(
    entry: &Value,
    since: usize,
    messages: &[Value],
    decision_after: Option<usize>,
) -> bool {
    let kind = entry.get("kind").and_then(Value::as_str).unwrap_or("");
    entry
        .get("evidence")
        .and_then(Value::as_array)
        .is_some_and(|refs| {
            refs.iter().any(|reference| {
                index(reference).is_ok_and(|index| {
                    (index >= since
                        || (kind == "decision" && decision_after.is_some_and(|at| index > at)))
                        && messages.get(index).is_some_and(|message| {
                            (kind == "decision"
                                && message.get("role").and_then(Value::as_str) == Some("user"))
                                || (message.get("role").and_then(Value::as_str) == Some("tool")
                                    && message
                                        .get("content")
                                        .and_then(Value::as_str)
                                        .and_then(|text| json::parse(text).ok())
                                        .is_some_and(|result| {
                                            successful(&result)
                                                && primary(
                                                    kind,
                                                    message,
                                                    &result,
                                                    &messages[..index],
                                                )
                                        }))
                        })
                })
            })
        })
}

pub(super) fn entries<'a>(
    value: &'a Value,
    key: &str,
    messages: &[Value],
) -> Result<Vec<&'a Value>, String> {
    let entries = value
        .get(key)
        .and_then(Value::as_array)
        .ok_or_else(|| format!("Continuity memory requires {key} as an array"))?;
    for (position, entry) in entries.iter().enumerate() {
        let field = format!("{key}[{position}]");
        if !matches!(entry, Value::Object(_)) {
            return Err(format!(
                "{field} must be an object with description, kind and evidence; a label string alone cannot prove completed or resolved work"
            ));
        }
        nonempty(entry, "description").map_err(|error| format!("{field}: {error}"))?;
        let kind = nonempty(entry, "kind").map_err(|error| format!("{field}: {error}"))?;
        if !["change", "check", "inspection", "decision"].contains(&kind) {
            return Err(format!(
                "{field}.kind must be change, check, inspection or decision"
            ));
        }
        let refs = entry
            .get("evidence")
            .and_then(Value::as_array)
            .filter(|refs| !refs.is_empty())
            .ok_or_else(|| {
                format!("{field}.evidence must be a nonempty array of history:N references")
            })?;
        let mut has_primary = false;
        for reference in refs {
            let at = index(reference).map_err(|error| format!("{field}.evidence: {error}"))?;
            let message = messages
                .get(at)
                .ok_or_else(|| format!("{field}.evidence refers to nonexistent history:{at}"))?;
            let role = message.get("role").and_then(Value::as_str);
            if kind == "decision" {
                if role != Some("user") {
                    return Err(format!(
                        "{field}: A decision requires original user evidence; history:{at} is {}",
                        role.unwrap_or("unknown role")
                    ));
                }
                has_primary = true;
                continue;
            }
            if role != Some("tool") {
                return Err(format!(
                    "{field} references history:{at} ({}); cite only executed tool-result messages for change/check/inspection, not user requests or assistant tool calls",
                    role.unwrap_or("unknown role")
                ));
            }
            let result = message
                .get("content")
                .and_then(Value::as_str)
                .and_then(|text| json::parse(text).ok())
                .ok_or_else(|| format!("{field}: history:{at} tool result is not structured"))?;
            if result.get("cancelled") == Some(&Value::Bool(true))
                || result.get("timed_out") == Some(&Value::Bool(true))
                || result.get("outcome").and_then(Value::as_str) == Some("unknown")
                || (kind != "inspection" && !successful(&result))
            {
                return Err(format!(
                    "{field}: history:{at} cannot prove completed {kind} work: failed, cancelled or unknown result. Record known failures as inspections and keep unresolved work in remaining."
                ));
            }
            has_primary |= primary(kind, message, &result, &messages[..at]);
        }
        if !has_primary {
            let reason = match kind {
                "check" => {
                    "A completed check requires successful bash check:true evidence; ordinary shell exit 0 is not a recorded check"
                }
                _ => "A completed change requires write/edit or command evidence",
            };
            let cited = refs
                .iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
                .join(", ");
            return Err(format!(
                "{key}[{position}] has no primary {kind} evidence in [{cited}]. {reason}"
            ));
        }
    }
    Ok(entries.iter().collect())
}

fn primary(kind: &str, message: &Value, result: &Value, previous: &[Value]) -> bool {
    match kind {
        "check" => {
            result.get("check") == Some(&Value::Bool(true))
                && result.get("check_status").and_then(Value::as_str) == Some("passed")
                && is_check_call(message, previous)
        }
        "change" => result.get("bytes_written").is_some() || result.get("exit_code").is_some(),
        "inspection" => true,
        _ => false,
    }
}

fn successful(result: &Value) -> bool {
    result.get("error").is_none()
        && result.get("cancelled") != Some(&Value::Bool(true))
        && result.get("timed_out") != Some(&Value::Bool(true))
        && result.get("outcome").and_then(Value::as_str) != Some("unknown")
        && result
            .get("exit_code")
            .is_none_or(|exit| exit.as_usize() == Some(0))
}

fn is_check_call(result: &Value, previous: &[Value]) -> bool {
    let Some(id) = result.get("tool_call_id").and_then(Value::as_str) else {
        return false;
    };
    previous.iter().rev().any(|message| {
        message.get("role").and_then(Value::as_str) == Some("assistant")
            && message
                .get("tool_calls")
                .and_then(Value::as_array)
                .is_some_and(|calls| {
                    calls.iter().any(|call| {
                        call.get("id").and_then(Value::as_str) == Some(id)
                            && call.get("function").is_some_and(|function| {
                                function.get("name").and_then(Value::as_str) == Some("bash")
                                    && function
                                        .get("arguments")
                                        .and_then(Value::as_str)
                                        .and_then(|text| json::parse(text).ok())
                                        .is_some_and(|args| {
                                            args.get("check") == Some(&Value::Bool(true))
                                        })
                            })
                    })
                })
    })
}
