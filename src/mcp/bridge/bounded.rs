//! Keeps remote tool results within the inline tool-output budget; the full
//! result stays readable through the session output store.

use crate::{json::Value, output};

const LIMIT: usize = crate::tools::OUTPUT_LIMIT;
/// Room for the call's identity, outcome and notice fields.
const PREVIEW: usize = LIMIT - 2048;

/// Replaces binary payloads with placeholders; a result still over the limit
/// keeps a text preview inline. Either way the full result is saved.
pub(super) fn bound(value: Value, outputs: Option<&output::Store>) -> Value {
    let Value::Object(mut fields) = value else {
        return value;
    };
    let Some(full) = fields.remove("result") else {
        return Value::Object(fields);
    };
    let mut inline = full.clone();
    let omitted = omit_binary(&mut inline);
    let truncated = inline.encode().len() > LIMIT;
    if truncated {
        inline = preview(&inline);
    }
    if omitted || truncated {
        match outputs.map(|outputs| save(outputs, &full)) {
            Some(Ok(reference)) => {
                fields.insert("result_file".into(), Value::string(reference));
            }
            Some(Err(error)) => {
                fields.insert("result_file_error".into(), Value::string(error));
            }
            None => {}
        }
    }
    if truncated {
        fields.insert("truncated".into(), Value::Bool(true));
        fields.insert(
            "notice".into(),
            Value::string(
                "The remote result exceeded the inline limit; only a text preview is shown. Read result_file with offset and limit for the full result.",
            ),
        );
    }
    fields.insert("result".into(), inline);
    Value::Object(fields)
}

/// Image, audio and blob data never enter model context inline.
fn omit_binary(result: &mut Value) -> bool {
    let Some(Value::Array(blocks)) = (match result {
        Value::Object(fields) => fields.get_mut("content"),
        _ => None,
    }) else {
        return false;
    };
    let mut omitted = false;
    for block in blocks {
        let Value::Object(fields) = block else {
            continue;
        };
        let kind = fields
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned();
        let target = match kind.as_str() {
            "image" | "audio" => Some(fields),
            "resource" => match fields.get_mut("resource") {
                Some(Value::Object(resource)) => Some(resource),
                _ => None,
            },
            _ => None,
        };
        let Some(target) = target else {
            continue;
        };
        let key = if kind == "resource" { "blob" } else { "data" };
        let Some(Value::String(data)) = target.get(key) else {
            continue;
        };
        let mime = target
            .get("mimeType")
            .and_then(Value::as_str)
            .unwrap_or("application/octet-stream");
        let bytes = data.trim_end_matches('=').len() * 3 / 4;
        let placeholder = format!("[omitted {mime} data: {bytes} bytes; see result_file]");
        target.insert(key.into(), Value::string(placeholder));
        omitted = true;
    }
    omitted
}

fn preview(result: &Value) -> Value {
    let texts = result
        .get("content")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|block| {
            block
                .get("text")
                .or_else(|| block.get("resource").and_then(|r| r.get("text")))
                .and_then(Value::as_str)
        })
        .collect::<Vec<_>>();
    let text = if texts.is_empty() {
        result.encode()
    } else {
        texts.join("\n\n")
    };
    let mut fields = std::collections::BTreeMap::new();
    fields.insert(
        "content".into(),
        Value::Array(vec![Value::object([
            ("type", Value::string("text")),
            ("text", Value::string(excerpt(&text, PREVIEW))),
        ])]),
    );
    if let Some(error) = result.get("isError") {
        fields.insert("isError".into(), error.clone());
    }
    Value::Object(fields)
}

/// The longest prefix whose JSON encoding fits the budget.
fn excerpt(text: &str, budget: usize) -> String {
    let mut end = text.len().min(budget);
    loop {
        let candidate = &text[..text.floor_char_boundary(end)];
        let size = Value::string(candidate).encode().len();
        if size <= budget || candidate.is_empty() {
            return candidate.into();
        }
        end = candidate.len() * budget / size;
    }
}

fn save(outputs: &output::Store, result: &Value) -> Result<String, String> {
    let mut logs = outputs.create()?;
    let failed = |error: std::io::Error| format!("Could not save MCP result: {error}");
    logs.stdout
        .write(result.pretty().as_bytes())
        .map_err(failed)?;
    logs.stdout.finish().map_err(failed)?;
    logs.stderr.finish().map_err(failed)?;
    Ok(logs.stdout_ref)
}
