use crate::json::{self, Value};
use std::hash::Hasher;

/// Scoped labels let a model resolve previous work without retyping its exact wording.
pub fn prompt_view(text: &str) -> String {
    let Ok(mut value) = json::parse(text) else {
        return text.into();
    };
    if let Value::Object(fields) = &mut value {
        for key in ["constraints", "remaining"] {
            if let Some(Value::Array(items)) = fields.get_mut(key) {
                for item in items.iter_mut() {
                    if let Some(text) = item.as_str() {
                        *item = Value::string(format!("{} {text}", label(key, text)));
                    }
                }
            }
        }
    }
    value.encode()
}

pub(super) fn label(key: &str, text: &str) -> String {
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    hash.write(text.as_bytes());
    format!("@{key}:h{:016x}", hash.finish())
}

pub fn expand(candidate: &mut Value, previous: &Value) -> Result<(), String> {
    if let Value::Object(fields) = candidate {
        for key in ["constraints", "remaining"] {
            if let Some(Value::Array(items)) = fields.get_mut(key) {
                for item in items {
                    expand_text(item, previous)?;
                }
            }
        }
        for key in ["completed", "resolved"] {
            if let Some(Value::Array(entries)) = fields.get_mut(key) {
                for entry in entries {
                    if let Value::Object(fields) = entry
                        && let Some(description) = fields.get_mut("description")
                    {
                        expand_resolution(description, previous)?;
                    }
                }
            }
        }
    }
    Ok(())
}

/// An unusable label cannot resolve work or withdraw a constraint. Keep the
/// original items and let the caller record the uncertainty explicitly.
pub fn reconcile(candidate: &mut Value, previous: &Value) -> Result<Vec<String>, String> {
    let mut notices = Vec::new();
    let unusable = |value: &Value, notices: &mut Vec<String>, resolution: bool| {
        let Some(text) = value
            .as_str()
            .filter(|text| text.starts_with("@constraints:") || text.starts_with("@remaining:"))
        else {
            return false;
        };
        let token = text.split_whitespace().next().unwrap();
        let mut expanded = value.clone();
        let stable = token
            .split_once(':')
            .is_some_and(|(_, id)| id.starts_with('h'));
        let checked = if resolution {
            expand_resolution(&mut expanded, previous)
        } else {
            expand_text(&mut expanded, previous)
        };
        if stable && checked.is_ok() {
            return false;
        }
        notices.push(token.into());
        true
    };
    if let Value::Object(fields) = candidate {
        for key in ["constraints", "remaining"] {
            if let Some(Value::Array(items)) = fields.get_mut(key) {
                items.retain(|item| !unusable(item, &mut notices, false));
            }
        }
        for key in ["completed", "resolved"] {
            if let Some(Value::Array(entries)) = fields.get_mut(key) {
                entries.retain(|entry| {
                    !entry
                        .get("description")
                        .is_some_and(|description| unusable(description, &mut notices, true))
                });
            }
        }
    }
    expand(candidate, previous)?;
    notices.sort();
    notices.dedup();
    Ok(notices)
}

/// An explicit stable ID identifies an old item regardless of surrounding prose.
/// No paraphrase matching, prefix matching or ambiguous multi-item resolution.
fn expand_resolution(value: &mut Value, previous: &Value) -> Result<(), String> {
    let Some(text) = value.as_str() else {
        return Ok(());
    };
    let mut targets = Vec::new();
    let word = |ch: char| ch.is_alphanumeric() || ch == '_';
    for key in ["constraints", "remaining"] {
        for item in previous.get(key).and_then(Value::as_array).unwrap_or(&[]) {
            let Some(original) = item.as_str() else {
                continue;
            };
            let id = label(key, original);
            if text.match_indices(&id).any(|(at, _)| {
                !text[..at].chars().next_back().is_some_and(word)
                    && !text[at + id.len()..].chars().next().is_some_and(word)
            }) && !targets.contains(&(key, item))
            {
                targets.push((key, item));
            }
        }
    }
    match targets.as_slice() {
        [] => expand_text(value, previous),
        [(_, original)] => {
            *value = (*original).clone();
            Ok(())
        }
        _ => Err(
            "A completed/resolved entry references multiple prior items; its target is ambiguous"
                .into(),
        ),
    }
}

fn expand_text(value: &mut Value, previous: &Value) -> Result<(), String> {
    let Some(text) = value.as_str() else {
        return Ok(());
    };
    if !text.starts_with("@constraints:") && !text.starts_with("@remaining:") {
        return Ok(());
    }
    let label = text.split_whitespace().next().unwrap();
    let (key, id) = label.trim_start_matches('@').split_once(':').unwrap();
    let items = previous.get(key).and_then(Value::as_array).unwrap_or(&[]);
    let original = if id.starts_with('h') {
        let matches = items.iter().filter(|item| item.as_str().is_some_and(|text| self::label(key, text) == label)).collect::<Vec<_>>();
        matches.first().copied().filter(|first| matches.iter().all(|item| *item == *first))
    } else {
        // Older scoped proposals used array indices; persisted descriptions
        // are still exact text and need no migration.
        id.parse::<usize>().ok().and_then(|index| items.get(index))
    }.filter(|item| item.as_str().is_some()).ok_or_else(|| format!("Unknown or ambiguous previous-memory reference {label}; use the labels shown in previous continuity memory"))?;
    *value = original.clone();
    Ok(())
}

#[cfg(test)]
mod tests;
