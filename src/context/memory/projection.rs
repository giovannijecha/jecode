use crate::json::{self, Value};

/// The summary proposes additions; completed work is a read-only native archive.
/// Open work and constraints keep their original text and review labels.
pub fn summary_input(text: &str, limit: usize) -> String {
    let view = active_view(text, limit);
    let Ok(Value::Object(mut fields)) = json::parse(&view) else {
        return view;
    };
    let Some(Value::Array(mut completed)) = fields.remove("completed") else {
        return view;
    };
    if completed.is_empty() {
        return view;
    }
    let count = json::parse(text)
        .ok()
        .and_then(|value| {
            value
                .get("completed")
                .and_then(Value::as_array)
                .map(<[Value]>::len)
        })
        .unwrap_or(completed.len());
    for entry in &mut completed {
        if let Value::Object(entry) = entry
            && let Some(Value::String(description)) = entry.get_mut("description")
            && description.len() > 160
        {
            *description = super::shorten(description, 160);
        }
    }
    fields.insert(
        "completed_archive".into(),
        Value::object([
            ("reference", Value::string("history:memory")),
            ("count", Value::number(count)),
            (
                "omitted",
                Value::number(count.saturating_sub(completed.len())),
            ),
            ("retained", Value::Array(completed)),
        ]),
    );
    fields.insert("completed".into(), Value::Array(vec![]));
    Value::Object(fields).encode()
}

/// Bound the active view, retaining the complete validated ledger separately.
/// Constraints, open work and resolutions are never shortened or archived.
pub fn active_view(text: &str, limit: usize) -> String {
    if text.len() <= limit {
        return text.into();
    }
    let Ok(Value::Object(mut fields)) = json::parse(text) else {
        return text.into();
    };
    let Some(Value::Array(mut completed)) = fields.remove("completed") else {
        return text.into();
    };
    let count = completed.len();
    if count == 0 {
        return text.into();
    }
    completed.sort_by_key(|entry| {
        entry
            .get("evidence")
            .and_then(Value::as_array)
            .unwrap_or(&[])
            .iter()
            .filter_map(|reference| {
                reference
                    .as_str()?
                    .strip_prefix("history:")?
                    .parse::<usize>()
                    .ok()
            })
            .max()
            .unwrap_or(0)
    });
    let inline = count.min(16);
    completed = completed[count - inline..].to_vec();
    fields.remove("completed_archive");
    fields.insert(
        "completed_archive".into(),
        Value::object([
            ("reference", Value::string("history:memory")),
            ("count", Value::number(count)),
            ("omitted", Value::number(count - inline)),
        ]),
    );
    for cap in [512, 256, 160, 96, 64] {
        for entry in &mut completed {
            if let Value::Object(entry) = entry
                && let Some(Value::String(description)) = entry.get_mut("description")
                && description.len() > cap
            {
                *description = super::shorten(description, cap);
            }
        }
        fields.insert("completed".into(), Value::Array(completed.clone()));
        let view = Value::Object(fields.clone()).encode();
        if view.len() <= limit {
            return view;
        }
    }
    let render = |retained: usize| {
        let mut fields = fields.clone();
        fields.insert(
            "completed".into(),
            Value::Array(completed[inline - retained..].to_vec()),
        );
        fields.insert(
            "completed_archive".into(),
            Value::object([
                ("reference", Value::string("history:memory")),
                ("count", Value::number(count)),
                ("omitted", Value::number(count - retained)),
            ]),
        );
        Value::Object(fields).encode()
    };
    let (mut low, mut high) = (0usize, inline);
    while low < high {
        let middle = low + (high - low).div_ceil(2);
        if render(middle).len() <= limit {
            low = middle;
        } else {
            high = middle - 1;
        }
    }
    render(low)
}

#[cfg(test)]
mod tests;
