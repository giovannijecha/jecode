use crate::json::{self, Value};

pub(super) fn requests(messages: &[Value], before: usize, budget: usize) -> Option<Value> {
    let requests = messages
        .iter()
        .enumerate()
        .take(before)
        .filter(|(_, message)| message.get("role").and_then(Value::as_str) == Some("user"))
        .collect::<Vec<_>>();
    if requests.is_empty() {
        return None;
    }
    let quota = (budget / requests.len()).clamp(160, budget.max(160));
    let latest_index = messages
        .iter()
        .rposition(|message| message.get("role").and_then(Value::as_str) == Some("user"))
        .unwrap();
    let label = |index| {
        if index == latest_index {
            format!("history:{index} — current original user request:\n")
        } else {
            format!("history:{index} — archived user request:\n")
        }
    };
    let mut entries = Vec::new();
    let (first_index, first) = requests[0];
    let first_text = first.get("content").and_then(Value::as_str).unwrap_or("");
    let first_label = label(first_index);
    let first_entry = format!(
        "{first_label}{}",
        excerpt(first_text, (budget / 3).max(quota)),
    );
    let mut used = first_entry.len();
    entries.push((first_index, first_entry));
    for &(index, message) in requests.iter().skip(1).rev() {
        let text = message.get("content").and_then(Value::as_str).unwrap_or("");
        let label = label(index);
        // Preserve the current request in full when the existing preview budget
        // permits it, rather than assigning it the same quota as old requests.
        let limit = if index == latest_index {
            budget.saturating_sub(used + label.len())
        } else {
            quota
        };
        let entry = format!("{label}{}", excerpt(text, limit));
        if used + entry.len() > budget && !entries.is_empty() {
            break;
        }
        used += entry.len();
        entries.push((index, entry));
    }
    let omitted = if entries.len() < requests.len() {
        "Some intermediate requests are omitted from this view.\n\n"
    } else {
        ""
    };
    entries.sort_by_key(|(index, _)| *index);
    let location = if latest_index >= before {
        "retained in the live transcript"
    } else {
        "included below"
    };
    Some(Value::object([
        ("role", Value::string("user")),
        (
            "content",
            Value::string(format!(
                "Original user request excerpts, in chronological order.\nLatest original user request: history:{latest_index} ({location}).\nComplete original request history: history:requests.\n\n{omitted}{}",
                entries
                    .into_iter()
                    .map(|(_, text)| text)
                    .collect::<Vec<_>>()
                    .join("\n\n")
            )),
        ),
    ]))
}

pub(super) fn preview(message: &Value, index: usize, limit: usize) -> Value {
    let Some(content) = message.get("content").and_then(Value::as_str) else {
        return message.clone();
    };
    if content.len() <= limit {
        return message.clone();
    }
    let mut result = json::parse(content)
        .unwrap_or_else(|_| Value::object([("content", Value::string(content))]));
    let Value::Object(fields) = &mut result else {
        return message.clone();
    };
    let mut shortened = false;
    for key in ["content", "stdout", "stderr"] {
        if let Some(Value::String(text)) = fields.get_mut(key)
            && text.len() > limit / 3
        {
            *text = excerpt(text, limit / 3);
            shortened = true;
        }
    }
    for key in ["file_changes", "file_observations", "file_protections"] {
        shortened |= preview_entries(fields, key, limit / 3);
    }
    if let Some(Value::Object(tracking)) = fields.get_mut("file_tracking") {
        shortened |= preview_entries(tracking, "errors", limit / 3);
    }
    if !shortened {
        return message.clone();
    }
    fields.insert("context_truncated".into(), Value::Bool(true));
    fields.insert(
        "history_reference".into(),
        Value::string(format!("history:{index}")),
    );
    fields.insert("context_notice".into(), Value::string("Only a context preview is shown. Original pagination fields describe the full tool result. Read history_reference to retrieve the original result; saved output references also remain readable."));
    let mut message = message.clone();
    if let Value::Object(fields) = &mut message {
        fields.insert("content".into(), Value::string(result.encode()));
    }
    message
}

fn preview_entries(
    fields: &mut std::collections::BTreeMap<String, Value>,
    key: &str,
    limit: usize,
) -> bool {
    let Some(Value::Array(entries)) = fields.get_mut(key) else {
        return false;
    };
    if entries
        .iter()
        .map(|entry| entry.encode().len())
        .sum::<usize>()
        <= limit
    {
        return false;
    }
    let count = entries.len();
    let mut bytes = 0;
    let keep = entries
        .iter()
        .take_while(|entry| {
            bytes += entry.encode().len();
            bytes <= limit
        })
        .count();
    entries.truncate(keep);
    fields.insert(format!("{key}_original_count"), Value::number(count));
    true
}

pub(super) fn excerpt(text: &str, limit: usize) -> String {
    if text.len() <= limit {
        return text.into();
    }
    let marker = "\n[... context excerpt; full original remains in history ...]\n";
    let available = limit.saturating_sub(marker.len());
    let mut head = available * 2 / 3;
    while !text.is_char_boundary(head) {
        head -= 1;
    }
    let mut tail = text.len().saturating_sub(available / 3);
    while !text.is_char_boundary(tail) {
        tail += 1;
    }
    format!("{}{marker}{}", &text[..head], &text[tail..])
}
