use crate::json::{self, Value};

/// A fixed checkpoint of the first and recent user requests before `before`.
/// Later messages never change this prefix; long requests retain history references.
pub(super) fn requests(messages: &[Value], before: usize, budget: usize) -> Option<Value> {
    let is_user = |message: &Value| message.get("role").and_then(Value::as_str) == Some("user");
    let requests = messages
        .iter()
        .enumerate()
        .take(before)
        .filter(|(_, message)| is_user(message))
        .collect::<Vec<_>>();
    let latest_index = requests.last()?.0;
    let mut entries = Vec::new();
    let mut used = 0;
    if requests.len() > 1 {
        let (index, message) = requests[0];
        let label = format!("history:{index} — archived user request:\n");
        let allowance = budget / 3;
        if allowance > label.len() {
            let text = crate::attachments::user_text(message);
            let entry = format!("{label}{}", excerpt(&text, allowance - label.len()));
            used = entry.len() + 2;
            entries.push((index, entry));
        }
    }
    for &(index, message) in requests.iter().rev() {
        if entries.first().is_some_and(|(first, _)| *first == index) {
            continue;
        }
        let text = crate::attachments::user_text(message);
        let text = text.as_str();
        let label = format!("history:{index} — archived user request:\n");
        let available = budget.saturating_sub(used + label.len());
        if available == 0 {
            break;
        }
        let entry = format!("{label}{}", excerpt(text, available));
        used += entry.len() + 2;
        entries.push((index, entry));
        if text.len() > available {
            break;
        }
    }
    if entries.is_empty() {
        return None;
    }
    let omitted = if entries.len() < requests.len() {
        "Earlier requests are omitted from this view.\n\n"
    } else {
        ""
    };
    entries.sort_by_key(|(index, _)| *index);
    Some(Value::object([
        ("role", Value::string("user")),
        (
            "content",
            Value::string(format!(
                "Archived original user requests, in chronological order.\nLast user request in this checkpoint: history:{latest_index}. Later user messages continue or update this work; an earlier request does not override a later correction or cancellation.\nComplete original request history: history:requests.\n\n{omitted}{}",
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

pub(crate) fn excerpt(text: &str, limit: usize) -> String {
    if text.len() <= limit {
        return text.into();
    }
    let marker = "\n[... context excerpt; full original remains in history ...]\n";
    if limit < marker.len() {
        return text[..text.floor_char_boundary(limit)].into();
    }
    let available = limit.saturating_sub(marker.len());
    let head = text.floor_char_boundary(available * 2 / 3);
    let tail = text.ceil_char_boundary(text.len().saturating_sub(available / 3));
    format!("{}{marker}{}", &text[..head], &text[tail..])
}
