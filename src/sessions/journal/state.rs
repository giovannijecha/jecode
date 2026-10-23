use crate::json::Value;

pub(super) fn without_partial(mut state: Value) -> Value {
    if let Value::Object(fields) = &mut state
        && let Some(Value::Object(pending)) = fields.get_mut("pending")
    {
        pending.remove("partial_text");
    }
    state
}

pub(super) fn delta(previous: Option<&Value>, next: &Value) -> Value {
    match (previous, next) {
        (Some(Value::Object(previous)), Value::Object(next)) => Value::Object(
            next.iter()
                .filter(|(key, value)| previous.get(*key) != Some(*value))
                .map(|(key, value)| (key.clone(), delta(previous.get(key), value)))
                .collect(),
        ),
        _ => next.clone(),
    }
}

pub(super) fn merge(previous: &mut Value, patch: &Value) {
    match (previous, patch) {
        (Value::Object(previous), Value::Object(patch)) => {
            for (key, value) in patch {
                if let Some(previous) = previous.get_mut(key) {
                    merge(previous, value);
                } else {
                    previous.insert(key.clone(), value.clone());
                }
            }
        }
        (previous, patch) => *previous = patch.clone(),
    }
}

/// Older v3 writers omitted an empty attachment list. A marker-free text edit
/// without that field cleared the draft's attachments; keep explicit lists and
/// edits that still contain attachment markers intact.
pub(super) fn clear_legacy_draft_attachments(state: &mut Value, patch: &Value) {
    let Some(draft) = patch.get("input").and_then(|input| input.get("draft")) else {
        return;
    };
    if draft.get("attachments").is_some()
        || draft
            .get("text")
            .and_then(Value::as_str)
            .is_none_or(|text| text.contains(crate::attachments::MARKER))
    {
        return;
    }
    let Value::Object(state) = state else {
        return;
    };
    if let Some(Value::Object(input)) = state.get_mut("input")
        && let Some(Value::Object(draft)) = input.get_mut("draft")
    {
        draft.insert("attachments".into(), Value::Array(vec![]));
    }
}

pub(super) fn partial_delta(previous: &str, next: &str) -> Value {
    let keep = if next.starts_with(previous) {
        previous.len()
    } else {
        previous
            .chars()
            .zip(next.chars())
            .take_while(|(left, right)| left == right)
            .map(|(character, _)| character.len_utf8())
            .sum()
    };
    Value::object([
        ("keep_bytes", Value::number(keep)),
        ("text", Value::string(&next[keep..])),
    ])
}

pub(super) fn apply_partial(previous: &mut String, patch: &Value) -> Result<(), String> {
    let keep = patch
        .get("keep_bytes")
        .and_then(Value::as_usize)
        .ok_or("Invalid partial response continuation")?;
    let text = patch
        .get("text")
        .and_then(Value::as_str)
        .ok_or("Invalid partial response text")?;
    if !previous.is_char_boundary(keep) {
        return Err("Partial response continuation is outside a UTF-8 boundary".into());
    }
    previous.truncate(keep);
    previous.push_str(text);
    Ok(())
}

pub(super) fn with_partial(mut state: Value, text: &str) -> Result<Value, String> {
    let Value::Object(fields) = &mut state else {
        return Err("Invalid journal state".into());
    };
    let Some(Value::Object(pending)) = fields.get_mut("pending") else {
        return Err("Missing journal pending state".into());
    };
    pending.insert("partial_text".into(), Value::string(text));
    Ok(state)
}
