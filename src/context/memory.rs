use crate::json::{self, Value};

mod evidence;
mod feedback;
mod pending;
mod projection;
mod proof;
use proof::{entries, has_new_evidence, reviewed_request};
mod references;
mod repair;
mod request;
pub use evidence::record as source_record;
pub use feedback::validation_feedback;
pub use projection::active_view;
pub use projection::summary_input;
pub use references::prompt_view;
pub use repair::has_update_facts as repair_has_update_facts;
pub use repair::sources as repair_sources;
pub use request::hint as request_review_hint;

pub const RESPONSE_SCHEMA: &str = r##"{"type":"object","required":["objective","constraints","completed","remaining","next_action"],"properties":{"objective":{"type":"string","minLength":1},"constraints":{"type":"array","items":{"type":"string","minLength":1}},"completed":{"type":"array","description":"New verified additions to the native completed ledger. Prior completed identities and proofs are retained automatically.","items":{"$ref":"#/$defs/proof"}},"remaining":{"type":"array","items":{"oneOf":[{"type":"string","minLength":1},{"$ref":"#/$defs/pending"}]}},"next_action":{"type":"string","minLength":1},"resolved":{"type":"array","items":{"$ref":"#/$defs/proof"}}},"$defs":{"pending":{"type":"object","required":["description"],"additionalProperties":false,"properties":{"description":{"type":"string","minLength":1},"kind":{"enum":["change","check","inspection","decision"]}}},"proof":{"type":"object","required":["description","kind","evidence"],"properties":{"description":{"type":"string","minLength":1},"kind":{"enum":["change","check","inspection","decision"]},"evidence":{"type":"array","minItems":1,"items":{"type":"string","pattern":"^history:[0-9]+$"}}}}}}"##;

pub fn prompt() -> String {
    format!("Continuity memory response interface (JSON Schema):\n{RESPONSE_SCHEMA}\n\n{PROMPT}")
}

pub const PROMPT: &str = r#"Prepare continuity memory from the original requests and new transcript, without executing the task.
The response updates a harness-owned ledger. objective and next_action describe the latest user goal and next unfinished step. constraints are continuing restrictions; remaining is unfinished tasks and reporting work.

Existing constraints, completed proof and pending work are retained automatically after any required request review. Empty arrays add nothing. Native requirement review names prior labels requiring explicit retain/resolve classification for a new user request. Previous entries have stable @constraints:h... and @remaining:h... labels; an exact label identifies its original text. A completed entry can close an old pending item with new matching primary evidence. resolved entries can withdraw old work or restrictions with an original user decision, or close work with new tool evidence.

Each original record exposes eligible proof kinds and a history reference. Tool results prove actions/checks; user messages prove decisions. An assistant proposal is not execution proof. A write is not a check. Failed, cancelled and unknown actions remain unfinished.
Transcript portions can omit earlier or later work. Keep exact paths, commands and uncertainty. Native retention, proof freshness and request review validate the proposed ledger."#;

/// Retention is owned by the harness; the model proposes additions and resolutions.
#[cfg(test)]
pub fn prepare(
    text: &str,
    previous: &str,
    messages: &[Value],
    since: usize,
    limit: usize,
) -> Result<String, String> {
    prepare_with_notices(text, previous, messages, since, limit).map(|(memory, _)| memory)
}

pub fn prepare_with_notices(
    text: &str,
    previous: &str,
    messages: &[Value],
    since: usize,
    limit: usize,
) -> Result<(String, Vec<String>), String> {
    let (candidate, old, notices) = proposal(text, previous)?;
    request::review(&candidate, &old, messages, since)?;
    assemble(candidate, old, notices, previous, messages, since, limit)
}

fn proposal(text: &str, previous: &str) -> Result<(Value, Value, Vec<String>), String> {
    let mut candidate = json::parse(text.trim()).map_err(|_| "Continuity memory must be a JSON object with objective, constraints, completed, remaining and next_action")?;
    pending::normalize(&mut candidate)?;
    let old = json::parse(previous).unwrap_or(Value::Null);
    if let Value::Object(fields) = &mut candidate
        && !fields.contains_key("completed")
        && old
            .get("completed")
            .and_then(Value::as_array)
            .is_some_and(|items| !items.is_empty())
    {
        // An omitted addition cannot erase the harness-owned completed ledger.
        // Carry its exact proof below; the assembled memory still passes the
        // same full validation. Wrong types and missing initial ledgers fail.
        fields.insert("completed".into(), Value::Array(vec![]));
    }
    let notices = references::reconcile(&mut candidate, &old)?;
    Ok((candidate, old, notices))
}

fn assemble(
    mut candidate: Value,
    old: Value,
    notices: Vec<String>,
    previous: &str,
    messages: &[Value],
    since: usize,
    limit: usize,
) -> Result<(String, Vec<String>), String> {
    if let Value::Object(fields) = &mut candidate
        && let Some(Value::Array(entries)) = fields.get_mut("completed")
    {
        for entry in entries {
            if let Some(original) = old
                .get("completed")
                .and_then(Value::as_array)
                .unwrap_or(&[])
                .iter()
                .find(|old| retains_completed(old, &[entry]))
                && let Value::Object(fields) = entry
            {
                fields.insert(
                    "description".into(),
                    original.get("description").unwrap().clone(),
                );
            }
        }
    }
    if !notices.is_empty() {
        let review = "Reconcile unresolved continuity references against the original user requests and recorded tool results before reporting completion.";
        append_unique(&mut candidate, "remaining", &Value::string(review));
        if let Value::Object(fields) = &mut candidate {
            fields.insert("next_action".into(), Value::string(review));
        }
    }
    if let Ok(old) = json::parse(previous)
        && strings(&candidate, "constraints").is_ok()
        && strings(&candidate, "remaining").is_ok()
        && candidate
            .get("completed")
            .and_then(Value::as_array)
            .is_some()
    {
        let resolved = candidate
            .get("resolved")
            .and_then(Value::as_array)
            .unwrap_or(&[])
            .to_vec();
        let completed = candidate
            .get("completed")
            .and_then(Value::as_array)
            .unwrap()
            .to_vec();
        for key in ["constraints", "remaining"] {
            if let Some(items) = old.get(key).and_then(Value::as_array) {
                for item in items {
                    let resolved_item = resolved
                        .iter()
                        .chain(if key == "remaining" {
                            &completed[..]
                        } else {
                            &[]
                        })
                        .any(|entry| {
                            entry.get("description") == Some(item)
                                && (key == "remaining"
                                    || entry.get("kind").and_then(Value::as_str)
                                        == Some("decision"))
                                && has_new_evidence(
                                    entry,
                                    since,
                                    messages,
                                    reviewed_request(&old, messages),
                                )
                        });
                    if resolved_item {
                        // A model can repeat an item in both arrays. New primary
                        // proof closes that exact old item; copying it back into
                        // pending work must not undo the verified resolution.
                        if let Value::Object(fields) = &mut candidate
                            && let Some(Value::Array(items)) = fields.get_mut(key)
                        {
                            items.retain(|candidate| candidate != item);
                        }
                    } else {
                        append_unique(&mut candidate, key, item);
                    }
                }
            }
        }
        if let Some(items) = old.get("completed").and_then(Value::as_array) {
            for item in items {
                if !retains_completed(item, &completed.iter().collect::<Vec<_>>()) {
                    append_unique(&mut candidate, "completed", item);
                }
            }
        }
    }
    // The durable ledger retains full completed identities and proofs. Only its
    // model projection is shortened or archived; open work is never removed.
    if let Value::Object(fields) = &mut candidate {
        fields.remove("completed_archive");
    }
    request::stamp(&mut candidate, &old, messages, since);
    validate(&candidate.encode(), previous, messages, since, limit).map(|valid| (valid, notices))
}

fn append_unique(value: &mut Value, key: &str, item: &Value) {
    if let Value::Object(fields) = value
        && let Some(Value::Array(items)) = fields.get_mut(key)
        && !items.contains(item)
    {
        items.push(item.clone());
    }
}

fn shorten(description: &str, limit: usize) -> String {
    if description.len() <= limit {
        return description.into();
    }
    let marker = " ... (see evidence)";
    let mut end = limit - marker.len();
    while !description.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}{marker}", &description[..end])
}

fn retains_completed(old: &Value, entries: &[&Value]) -> bool {
    entries.iter().any(|entry| {
        entry.get("kind") == old.get("kind")
            && old
                .get("description")
                .and_then(Value::as_str)
                .is_some_and(|text| {
                    entry
                        .get("description")
                        .and_then(Value::as_str)
                        .is_some_and(|description| {
                            description == text
                                || [512, 256, 160, 96, 64]
                                    .iter()
                                    .any(|limit| description == shorten(text, *limit))
                        })
                })
            && old
                .get("evidence")
                .and_then(Value::as_array)
                .is_some_and(|refs| {
                    entry
                        .get("evidence")
                        .and_then(Value::as_array)
                        .is_some_and(|evidence| {
                            refs.iter().all(|reference| evidence.contains(reference))
                        })
                })
    })
}

pub fn validate(
    text: &str,
    previous: &str,
    messages: &[Value],
    since: usize,
    limit: usize,
) -> Result<String, String> {
    let text = text.trim();
    if active_view(text, limit).len() > limit {
        return Err(format!(
            "Continuity memory exceeds {limit} UTF-8 bytes; shorten descriptions and retain evidence references"
        ));
    }
    let value = json::parse(text).map_err(|_| "Continuity memory must be a JSON object with objective, constraints, completed, remaining and next_action")?;
    if !matches!(value, Value::Object(_)) {
        return Err("Continuity memory must be a JSON object".into());
    }
    nonempty(&value, "objective")?;
    nonempty(&value, "next_action")?;
    let constraints = strings(&value, "constraints")?;
    let remaining = strings(&value, "remaining")?;
    let completed = entries(&value, "completed", messages)?;
    let resolved = if value.get("resolved").is_some() {
        entries(&value, "resolved", messages)?
    } else {
        vec![]
    };
    if let Ok(previous) = json::parse(previous) {
        if let Ok(old_constraints) = strings(&previous, "constraints") {
            for constraint in old_constraints {
                if !constraints.contains(&constraint)
                    && !resolved.iter().any(|entry| {
                        entry.get("description").and_then(Value::as_str) == Some(constraint)
                            && entry.get("kind").and_then(Value::as_str) == Some("decision")
                            && has_new_evidence(
                                entry,
                                since,
                                messages,
                                reviewed_request(&previous, messages),
                            )
                    })
                {
                    return Err(format!(
                        "A previous constraint disappeared without a new user decision: {constraint}"
                    ));
                }
            }
        }
        if let Ok(old_remaining) = strings(&previous, "remaining") {
            for item in old_remaining {
                if !remaining.contains(&item)
                    && !completed.iter().chain(&resolved).any(|entry| {
                        entry.get("description").and_then(Value::as_str) == Some(item)
                            && has_new_evidence(
                                entry,
                                since,
                                messages,
                                reviewed_request(&previous, messages),
                            )
                    })
                {
                    return Err(format!(
                        "An unfinished item disappeared without new evidence: {item}"
                    ));
                }
            }
        }
        if let Some(old_completed) = previous.get("completed").and_then(Value::as_array) {
            for old in old_completed {
                if let Some(references) = old.get("evidence").and_then(Value::as_array) {
                    for reference in references {
                        if !completed.iter().any(|entry| {
                            entry
                                .get("evidence")
                                .and_then(Value::as_array)
                                .is_some_and(|refs| refs.contains(reference))
                        }) {
                            return Err(format!(
                                "Previous completed evidence disappeared: {}",
                                reference.encode()
                            ));
                        }
                    }
                }
                if !retains_completed(old, &completed) {
                    return Err("Previous completed work was rewritten; retain its kind, description and evidence (owned description shortening is allowed)".into());
                }
            }
        }
    }
    Ok(value.encode())
}

fn nonempty<'a>(value: &'a Value, key: &str) -> Result<&'a str, String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|text| !text.trim().is_empty())
        .ok_or_else(|| format!("Continuity memory requires nonempty {key}"))
}

fn strings<'a>(value: &'a Value, key: &str) -> Result<Vec<&'a str>, String> {
    value
        .get(key)
        .and_then(Value::as_array)
        .ok_or_else(|| format!("Continuity memory requires {key} as an array"))?
        .iter()
        .enumerate()
        .map(|(index, value)| {
            value
                .as_str()
                .filter(|text| !text.trim().is_empty())
                .ok_or_else(|| {
                    let actual = match value {
                        Value::Null => "null",
                        Value::Bool(_) => "boolean",
                        Value::Number(_) => "number",
                        Value::String(_) => "empty string",
                        Value::Array(_) => "array",
                        Value::Object(_) => "object",
                    };
                    format!("Continuity memory {key}[{index}] requires a nonempty string; received {actual}")
                })
        })
        .collect()
}

#[cfg(test)]
pub(crate) fn fixture(objective: &str) -> String {
    Value::object([
        ("objective", Value::string(objective)),
        ("constraints", Value::Array(vec![])),
        ("completed", Value::Array(vec![])),
        (
            "remaining",
            Value::Array(vec![Value::string("Continue verification")]),
        ),
        ("next_action", Value::string("Continue verification")),
    ])
    .encode()
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod delta_tests;

#[cfg(test)]
mod resolution_tests;

#[cfg(test)]
mod request_tests;

#[cfg(test)]
mod interface_tests;

#[cfg(test)]
mod pending_tests;
