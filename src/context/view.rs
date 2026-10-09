use crate::json::{self, Value};

/// Recent user requests before `before`, newest first within `budget`. They stay
/// verbatim; the request crossing the budget is shortened and older ones are omitted.
pub(super) fn requests(messages: &[Value], before: usize, budget: usize) -> Option<Value> {
    let is_user = |message: &Value| message.get("role").and_then(Value::as_str) == Some("user");
    let requests = messages
        .iter()
        .enumerate()
        .take(before)
        .filter(|(_, message)| is_user(message))
        .collect::<Vec<_>>();
    let latest_index = messages.iter().rposition(is_user)?;
    let mut entries = Vec::new();
    let mut used = 0;
    for &(index, message) in requests.iter().rev() {
        let text = crate::attachments::provider::user_text(message);
        let text = text.as_str();
        let label = if index == latest_index {
            format!("history:{index} — current original user request:\n")
        } else {
            format!("history:{index} — archived user request:\n")
        };
        let available = budget.saturating_sub(used + label.len());
        if available == 0 {
            break;
        }
        let entry = format!("{label}{}", excerpt(text, available));
        used += entry.len() + 2;
        entries.push(entry);
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
    entries.reverse();
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
                "Recent original user requests, in chronological order.\nLatest original user request: history:{latest_index} ({location}).\nComplete original request history: history:requests.\n\n{omitted}{}",
                entries.join("\n\n")
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
