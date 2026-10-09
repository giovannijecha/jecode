use super::{has_new_evidence, references, reviewed_request, strings};
use crate::json::Value;

fn latest(messages: &[Value]) -> Option<usize> {
    messages
        .iter()
        .rposition(|message| message.get("role").and_then(Value::as_str) == Some("user"))
}

fn unreviewed(previous: &Value, messages: &[Value], since: usize) -> Option<usize> {
    let reviewed = reviewed_request(previous, messages);
    latest(messages).filter(|request| {
        reviewed != Some(*request)
            && (*request >= since || reviewed.is_some_and(|at| *request > at))
    })
}

pub fn hint(previous: &str, messages: &[Value], since: usize) -> String {
    let Ok(previous) = crate::json::parse(previous) else {
        return String::new();
    };
    let Some(request) = unreviewed(&previous, messages, since) else {
        return String::new();
    };
    if previous
        .get("reviewed_request_history")
        .and_then(Value::as_usize)
        == Some(request)
        || ["constraints", "remaining"]
            .iter()
            .all(|key| strings(&previous, key).map_or(true, |items| items.is_empty()))
    {
        return String::new();
    }
    let labels = |key| {
        Value::Array(
            strings(&previous, key)
                .unwrap_or_default()
                .into_iter()
                .map(|item| Value::string(references::label(key, item)))
                .collect(),
        )
    };
    let state = Value::object([
        (
            "request_history",
            Value::string(format!("history:{request}")),
        ),
        ("automatic_carry", Value::Bool(false)),
        (
            "required_labels",
            Value::object([
                ("constraints", labels("constraints")),
                ("remaining", labels("remaining")),
            ]),
        ),
    ]);
    format!("Native requirement review:\n{}\n\n", state.encode())
}

/// At a new request, silent omission cannot decide which old requirements survive.
pub(super) fn review(
    candidate: &Value,
    previous: &Value,
    messages: &[Value],
    since: usize,
) -> Result<(), String> {
    let Some(request) = unreviewed(previous, messages, since) else {
        return Ok(());
    };
    if previous
        .get("reviewed_request_history")
        .and_then(Value::as_usize)
        == Some(request)
    {
        return Ok(());
    }
    let mut missing = Vec::new();
    for key in ["constraints", "remaining"] {
        let Ok(old) = strings(previous, key) else {
            continue;
        };
        let retained = strings(candidate, key)?;
        for item in old {
            let resolved = ["resolved", "completed"].into_iter().any(|field| {
                (key == "remaining" || field == "resolved")
                    && candidate
                        .get(field)
                        .and_then(Value::as_array)
                        .is_some_and(|entries| {
                            entries.iter().any(|entry| {
                                entry.get("description").and_then(Value::as_str) == Some(item)
                                    && (key == "remaining"
                                        || entry.get("kind").and_then(Value::as_str)
                                            == Some("decision"))
                                    && has_new_evidence(
                                        entry,
                                        since,
                                        messages,
                                        reviewed_request(previous, messages),
                                    )
                            })
                        })
            });
            if !retained.contains(&item) && !resolved {
                missing.push(references::label(key, item));
            }
        }
    }
    if missing.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "New user request history:{request} requires explicit review of prior requirements: {}. Each label requires a retained entry in its original array, or a completed/resolved entry with new primary evidence. Withdrawal of a constraint requires kind:decision with original user evidence.",
            missing.join(", ")
        ))
    }
}

/// The boundary is harness-owned; proposed metadata cannot bypass review.
pub(super) fn stamp(candidate: &mut Value, previous: &Value, messages: &[Value], since: usize) {
    if let Value::Object(fields) = candidate {
        fields.remove("reviewed_request_history");
        let request = latest(messages).filter(|request| {
            *request >= since
                || reviewed_request(previous, messages).is_some()
                || !matches!(previous, Value::Object(_))
        });
        if let Some(request) = request {
            fields.insert("reviewed_request_history".into(), Value::number(request));
        }
    }
}
