use super::required_string;
use crate::json::Value;

/// Files one edit call may change.
pub(crate) const MAX_FILES: usize = 12;

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

/// The files of a multi-file edit in call order, each entry with its path
/// and its own edits; `None` for a single-file call. The call's shape is
/// checked here; each entry's edits are checked when that file runs, so an
/// invalid entry fails alone.
pub(crate) fn files(arguments: &Value) -> Result<Option<Vec<(&str, &Value)>>, String> {
    let Some(files) = arguments.get("files") else {
        return Ok(None);
    };
    let single = arguments.get("path").is_some_and(|path| !placeholder(path));
    if placeholder(files) {
        return match files {
            Value::Array(_) if !single => Err(count()),
            _ => Ok(None),
        };
    }
    if single
        || ["edits", "old_text", "new_text"]
            .iter()
            .any(|name| arguments.get(name).is_some_and(|value| !placeholder(value)))
    {
        return Err("Use path with edits for one file, or files for several, not both".into());
    }
    let Value::Array(files) = files else {
        return Err("files must be an array of {path, edits} objects".into());
    };
    if files.len() > MAX_FILES {
        return Err(count());
    }
    let mut entries: Vec<(&str, &Value)> = Vec::with_capacity(files.len());
    for file in files {
        let path = file
            .get("path")
            .and_then(Value::as_str)
            .filter(|path| !path.is_empty())
            .ok_or("Each files entry must be an object with a path string and edits")?;
        if entries.iter().any(|(seen, _)| *seen == path) {
            return Err(format!(
                "files names {path} more than once; put all of a file's replacements in one entry"
            ));
        }
        entries.push((path, file));
    }
    Ok(Some(entries))
}

/// Whether a call takes the multi-file route, including a malformed `files`
/// that the route reports.
pub(crate) fn several(arguments: &Value) -> bool {
    !matches!(files(arguments), Ok(None))
}

fn count() -> String {
    format!("files must contain 1-{MAX_FILES} entries; split larger changes across calls")
}

/// Native calls may send unused fields empty.
fn placeholder(value: &Value) -> bool {
    match value {
        Value::Null => true,
        Value::String(text) => text.is_empty(),
        Value::Array(items) => items.is_empty(),
        _ => false,
    }
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
