use super::required_string;
use crate::json::Value;

/// Parses the edit tool's ordered replacements.
pub(crate) fn steps(arguments: &Value) -> Result<Vec<(&str, &str)>, String> {
    match arguments.get("edits") {
        Some(Value::Array(items)) if !items.is_empty() => items.iter().map(step).collect(),
        Some(Value::Array(_)) => Err("edits must contain at least one replacement".into()),
        _ => Err("edits must be an array".into()),
    }
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
            Value::Array(_) if !single => Err(EMPTY.into()),
            _ => Ok(None),
        };
    }
    // A path without replacements beside files changes nothing, so files is
    // the call; only single-file replacements conflict with it.
    if arguments
        .get("edits")
        .is_some_and(|value| !placeholder(value))
    {
        return Err(
            "Send replacements either as path with edits for one file, or inside files entries for several, not both"
                .into(),
        );
    }
    let Value::Array(files) = files else {
        return Err("files must be an array of {path, edits} objects".into());
    };
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

const EMPTY: &str = "files must contain at least one {path, edits} entry";

/// Native calls may send unused fields empty.
pub(crate) fn placeholder(value: &Value) -> bool {
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
