use super::required_string;
use crate::json::Value;

/// Parses the edit tool's ordered replacements, including legacy call shapes.
pub(crate) fn steps(arguments: &Value) -> Result<Vec<(&str, &str)>, String> {
    let legacy = ["old_text", "new_text"]
        .iter()
        .any(|name| match arguments.get(name) {
            None | Some(Value::Null) => false,
            Some(Value::String(text)) if text.is_empty() => false,
            Some(_) => true,
        });
    if let Some(items) = arguments.get("edits") {
        match items {
            Value::Array(items) if !items.is_empty() => {
                if legacy {
                    return Err("Use either edits or old_text/new_text, not both".into());
                }
                return items.iter().map(step).collect();
            }
            Value::Array(_) | Value::Null if legacy => return Ok(vec![step(arguments)?]),
            Value::Array(_) => return Err("edits must contain at least one replacement".into()),
            _ => return Err("edits must be an array".into()),
        }
    }
    Ok(vec![step(arguments)?])
}

fn step(arguments: &Value) -> Result<(&str, &str), String> {
    let old = required_string(arguments, "old_text")?;
    let new = required_string(arguments, "new_text")?;
    if old.is_empty() {
        return Err("old_text must not be empty; use write to replace the whole file".into());
    }
    if new.contains('\0') {
        return Err("File content must be UTF-8 text without NUL bytes".into());
    }
    Ok((old, new))
}
