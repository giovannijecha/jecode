use crate::{json::Value, output};

/// Protocol completeness and inline truncation are independent observations.
pub(super) fn bound(value: Value, outputs: &output::Store) -> Value {
    if value.encode().len() <= crate::tools::OUTPUT_LIMIT {
        return value;
    }
    let retained = save(outputs, &value);
    let Value::Object(mut fields) = value else {
        return Value::Null;
    };
    for key in ["locations", "diagnostics"] {
        if let Some(Value::Array(items)) = fields.get_mut(key) {
            let total = items.len();
            items.truncate(40);
            fields.insert(format!("total_{key}"), Value::number(total));
        }
    }
    fields.insert("truncated".into(), Value::Bool(true));
    fields.insert("inline_truncated".into(), Value::Bool(true));
    match retained {
        Ok(reference) => {
            fields.insert("result_file".into(), Value::string(reference));
        }
        Err(error) => {
            fields.insert("result_file_error".into(), Value::string(error));
        }
    }
    let mut result = Value::Object(fields);
    if result.encode().len() > crate::tools::OUTPUT_LIMIT {
        let preview = result.encode();
        let preview = excerpt(&preview, 32 * 1024);
        let mut envelope = Value::object([
            ("preview", Value::string(preview)),
            ("truncated", Value::Bool(true)),
            (
                "detail",
                Value::string(
                    "LSP output exceeded the inline budget; read result_file for the complete response.",
                ),
            ),
        ]);
        if let Value::Object(fields) = &mut envelope {
            for key in [
                "action",
                "status",
                "server",
                "server_name",
                "root",
                "freshness",
                "complete",
                "result_file",
                "result_file_error",
                "total_locations",
                "total_diagnostics",
            ] {
                if let Some(value) = result.get(key) {
                    let bounded = match value {
                        Value::String(text) => Value::string(excerpt(text, 2048)),
                        _ => value.clone(),
                    };
                    fields.insert(key.into(), bounded);
                }
            }
        }
        result = envelope;
    }
    result
}

fn excerpt(text: &str, budget: usize) -> String {
    let mut end = text.len().min(budget);
    loop {
        let candidate = &text[..text.floor_char_boundary(end)];
        let encoded = Value::string(candidate).encode().len();
        if encoded <= budget || candidate.is_empty() {
            return candidate.into();
        }
        end = candidate.len() * budget / encoded;
    }
}

fn save(outputs: &output::Store, value: &Value) -> Result<String, String> {
    let mut logs = outputs.create()?;
    logs.stdout
        .write(value.pretty().as_bytes())
        .map_err(|error| format!("Could not retain LSP result: {error}"))?;
    logs.stdout.finish().map_err(|error| error.to_string())?;
    logs.stderr.finish().map_err(|error| error.to_string())?;
    Ok(logs.stdout_ref)
}
