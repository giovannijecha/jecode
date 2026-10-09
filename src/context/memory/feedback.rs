use crate::json::{self, Value};
use std::collections::BTreeSet;

/// A rejected proposal and its native reference types, with no invented repair.
pub fn validation_feedback(
    candidate: &str,
    error: &str,
    previous: &str,
    messages: &[Value],
    since: usize,
    limit: usize,
) -> String {
    let parsed = (candidate.len() <= limit)
        .then(|| json::parse(candidate).ok())
        .flatten();
    let mut indices = BTreeSet::new();
    for field in ["completed", "resolved"] {
        for entry in parsed
            .as_ref()
            .and_then(|value| value.get(field))
            .and_then(Value::as_array)
            .unwrap_or(&[])
        {
            for reference in entry
                .get("evidence")
                .and_then(Value::as_array)
                .unwrap_or(&[])
            {
                if let Some(at) = reference
                    .as_str()
                    .and_then(|text| text.strip_prefix("history:"))
                    .filter(|text| {
                        !text.is_empty() && text.bytes().all(|byte| byte.is_ascii_digit())
                    })
                    .and_then(|text| text.parse::<usize>().ok())
                {
                    indices.insert(at);
                }
            }
        }
    }
    let feedback = Value::object([
        ("status", Value::string("rejected")),
        ("error", Value::string(error)),
        (
            "validation_errors",
            Value::Array(
                diagnostics(candidate, error, previous, messages, since, limit)
                    .into_iter()
                    .map(Value::string)
                    .collect(),
            ),
        ),
        ("previous_context_preserved", Value::Bool(true)),
        ("candidate_bytes", Value::number(candidate.len())),
        (
            "candidate",
            parsed.unwrap_or_else(|| {
                if candidate.len() <= limit {
                    Value::string(candidate)
                } else {
                    Value::Null
                }
            }),
        ),
        (
            "cited_sources",
            Value::Array(
                indices
                    .into_iter()
                    .map(|at| super::source_record(messages, at))
                    .collect(),
            ),
        ),
    ]);
    format!(
        "\n\nNative continuity validation result:\n{}",
        feedback.encode()
    )
}

// Independent native failures are visible in one repair; acceptance still uses prepare.
fn diagnostics(
    candidate: &str,
    error: &str,
    previous: &str,
    messages: &[Value],
    since: usize,
    limit: usize,
) -> Vec<String> {
    let mut errors = vec![error.to_owned()];
    let add = |errors: &mut Vec<String>, result: Result<(), String>| {
        if let Err(error) = result
            && !errors.contains(&error)
        {
            errors.push(error);
        }
    };
    if candidate.len() > limit {
        return errors;
    }
    let prepared = super::proposal(candidate, previous).ok();
    let raw = json::parse(candidate).ok();
    let Some(value) = prepared
        .as_ref()
        .map(|(value, _, _)| value)
        .or(raw.as_ref())
    else {
        return errors;
    };
    if !matches!(value, Value::Object(_)) {
        return errors;
    }
    for key in ["objective", "next_action"] {
        add(&mut errors, super::nonempty(value, key).map(|_| ()));
    }
    add(
        &mut errors,
        super::strings(value, "constraints").map(|_| ()),
    );
    if let Some((_, old, _)) = &prepared {
        add(&mut errors, super::strings(value, "remaining").map(|_| ()));
        add(
            &mut errors,
            super::request::review(value, old, messages, since),
        );
    }
    let carries_completed = json::parse(previous).ok().is_some_and(|value| {
        value
            .get("completed")
            .and_then(Value::as_array)
            .is_some_and(|items| !items.is_empty())
    });
    for key in ["completed", "resolved"] {
        if key == "resolved" && value.get(key).is_none() {
            continue;
        }
        if key == "completed" && value.get(key).is_none() && carries_completed {
            continue;
        }
        let Some(entries) = value.get(key).and_then(Value::as_array) else {
            add(
                &mut errors,
                super::entries(value, key, messages).map(|_| ()),
            );
            continue;
        };
        for (at, entry) in entries.iter().enumerate() {
            let single = Value::object([(key, Value::Array(vec![entry.clone()]))]);
            add(
                &mut errors,
                super::entries(&single, key, messages)
                    .map(|_| ())
                    .map_err(|error| {
                        error.replacen(&format!("{key}[0]"), &format!("{key}[{at}]"), 1)
                    }),
            );
        }
    }
    errors
}

#[cfg(test)]
mod tests;
