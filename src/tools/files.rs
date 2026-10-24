use super::required_string;
use crate::cancel::Cancellation;
use crate::checkpoints::session::{FileGuard, Session};
use crate::{file_change::MAX_DIFF_BYTES, json::Value};
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

mod mismatch;
mod page;
mod streaming;
pub(super) use page::read_page;
pub(crate) use page::read_source;
pub(crate) use page::read_text_page;

#[cfg(test)]
pub(super) fn read(
    root: &Path,
    arguments: &Value,
    cancellation: &crate::cancel::Cancellation,
) -> Result<Value, String> {
    let path = existing_path(root, required_string(arguments, "path")?)?;
    read_page(&path, arguments, cancellation)
}

#[cfg(test)]
pub(super) fn write(root: &Path, arguments: &Value) -> Result<Value, String> {
    write_with_change(root, arguments, None, &Cancellation::default()).map(|(result, _)| result)
}

pub(super) fn write_with_change(
    root: &Path,
    arguments: &Value,
    checkpoints: Option<&Session>,
    cancellation: &Cancellation,
) -> Result<(Value, Option<crate::file_change::Change>), String> {
    check_cancelled(cancellation)?;
    let path = writable_path(root, required_string(arguments, "path")?)?;
    let content = required_string(arguments, "content")?;
    let mut guard =
        checkpoints.map(|session| session.file_cancellable("write", &path, cancellation));
    // A write may replace non-text data; inability to read it cannot reject
    // an operation that was previously allowed.
    let previous = previous_text(&path, content.len());
    if let Some(guard) = &mut guard {
        guard.capture();
    }
    check_guard(guard.as_ref(), cancellation)?;
    let outcome = finish_replacement(guard, &path, content, cancellation);
    if let Err(error) = outcome.result {
        return Ok((failed(error, outcome.checkpoint), None));
    }
    let change =
        previous.and_then(|old| crate::file_change::Change::between(old.as_deref(), content));
    Ok((written(content.len() as u64, outcome.checkpoint), change))
}

#[cfg(test)]
pub(super) fn edit(root: &Path, arguments: &Value) -> Result<Value, String> {
    edit_with_change(root, arguments, None, &Cancellation::default()).map(|(result, _)| result)
}

pub(super) fn edit_with_change(
    root: &Path,
    arguments: &Value,
    checkpoints: Option<&Session>,
    cancellation: &Cancellation,
) -> Result<(Value, Option<crate::file_change::Change>), String> {
    check_cancelled(cancellation)?;
    let path = existing_path(root, required_string(arguments, "path")?)?;
    let edits = edit_steps(arguments)?;
    // Hold the path from reading to replacing so concurrent sessions take turns.
    let guard = checkpoints.map(|session| session.file_cancellable("edit", &path, cancellation));
    if fs::metadata(&path).is_ok_and(|metadata| metadata.len() > MAX_DIFF_BYTES as u64) {
        return streaming::edit(&path, &edits, guard, cancellation);
    }
    let content = read_text(&path, cancellation)?;
    let mut updated = content.clone();
    for (index, (old, new)) in edits.iter().enumerate() {
        check_cancelled(cancellation)?;
        let Some(position) = updated.find(old) else {
            return Ok((
                mismatch::result(
                    &content,
                    &edit_mismatch_error(
                        index,
                        "old text was not found",
                        "use exact current text from the supplied file context or read the file",
                    ),
                ),
                None,
            ));
        };
        // Include overlapping matches, which are ambiguous too.
        let next = position + updated[position..].chars().next().unwrap().len_utf8();
        if updated[next..].contains(old) {
            return Ok((
                mismatch::result(
                    &content,
                    &edit_mismatch_error(
                        index,
                        "old text occurs more than once",
                        "include more exact surrounding text from the supplied file context or read the file",
                    ),
                ),
                None,
            ));
        }
        updated.replace_range(position..position + old.len(), new);
    }
    if updated == content {
        return Ok((written(0, None), None));
    }
    let outcome = replace_protected(guard, &path, &content, &updated, cancellation);
    if let Err(error) = outcome.result {
        return Ok((failed(error, outcome.checkpoint), None));
    }
    let change = crate::file_change::Change::between(Some(&content), &updated);
    Ok((written(updated.len() as u64, outcome.checkpoint), change))
}

fn edit_mismatch_error(index: usize, reason: &str, initial_advice: &str) -> String {
    if index == 0 {
        format!("edit 1: {reason}; {initial_advice}")
    } else {
        format!(
            "edit {}: {reason}; matching uses content staged by preceding edits; no edits were saved, and current_content comes from the unchanged original file",
            index + 1
        )
    }
}

fn edit_steps(arguments: &Value) -> Result<Vec<(&str, &str)>, String> {
    let legacy = arguments.get("old_text").is_some() || arguments.get("new_text").is_some();
    if let Some(items) = arguments.get("edits") {
        if legacy {
            return Err("Use either edits or old_text/new_text, not both".into());
        }
        let items = items.as_array().ok_or("edits must be an array")?;
        if items.is_empty() {
            return Err("edits must contain at least one replacement".into());
        }
        return items.iter().map(edit_step).collect();
    }
    Ok(vec![edit_step(arguments)?])
}

fn edit_step(arguments: &Value) -> Result<(&str, &str), String> {
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

struct ReplacementOutcome {
    result: Result<(), String>,
    checkpoint: Option<Value>,
}

/// Saves the previous state before replacing the file. The replacement
/// proceeds whether or not saving succeeded; the result says which.
fn replace_protected(
    guard: Option<FileGuard<'_>>,
    path: &Path,
    expected: &str,
    content: &str,
    cancellation: &Cancellation,
) -> ReplacementOutcome {
    let Some(mut guard) = guard else {
        return finish_replacement(None, path, content, cancellation);
    };
    guard.capture();
    if let Err(error) = check_guard(Some(&guard), cancellation) {
        return ReplacementOutcome {
            result: Err(error),
            checkpoint: Some(guard.finish()),
        };
    }
    // An external writer does not honor our lease. Check the exact source
    // after capture, before installing the replacement derived from it.
    match read_text(path, cancellation) {
        Ok(current) if current == expected => {}
        Ok(_) => {
            return ReplacementOutcome {
                result: Err(
                    "The file changed while the edit was being prepared; read it and retry".into(),
                ),
                checkpoint: Some(guard.finish()),
            };
        }
        Err(error) => {
            return ReplacementOutcome {
                result: Err(error),
                checkpoint: Some(guard.finish()),
            };
        }
    }
    finish_replacement(Some(guard), path, content, cancellation)
}

fn finish_replacement(
    guard: Option<FileGuard<'_>>,
    path: &Path,
    content: &str,
    cancellation: &Cancellation,
) -> ReplacementOutcome {
    let Some(guard) = guard else {
        return ReplacementOutcome {
            result: replace_file(path, content, cancellation),
            checkpoint: None,
        };
    };
    let result = check_guard(Some(&guard), cancellation)
        .and_then(|()| replace_file(path, content, cancellation));
    let checkpoint = Some(guard.finish());
    ReplacementOutcome { result, checkpoint }
}

fn written(bytes: u64, checkpoint: Option<Value>) -> Value {
    let mut result = Value::object([("bytes_written", Value::number(bytes))]);
    if let (Value::Object(fields), Some(checkpoint)) = (&mut result, checkpoint) {
        fields.insert("checkpoint".into(), checkpoint);
    }
    result
}

fn failed(error: String, checkpoint: Option<Value>) -> Value {
    let mut result = Value::object([("error", Value::string(error))]);
    if let (Value::Object(fields), Some(checkpoint)) = (&mut result, checkpoint) {
        fields.insert("checkpoint".into(), checkpoint);
    }
    result
}

fn previous_text(path: &Path, updated_bytes: usize) -> Option<Option<String>> {
    if updated_bytes > MAX_DIFF_BYTES {
        return None;
    }
    match fs::metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Some(None),
        Ok(metadata) if metadata.is_file() && metadata.len() <= MAX_DIFF_BYTES as u64 => {}
        _ => return None,
    }
    let mut previous = String::new();
    // Bound the read too: another writer may grow the file after metadata.
    fs::File::open(path)
        .ok()?
        .take(MAX_DIFF_BYTES as u64 + 1)
        .read_to_string(&mut previous)
        .ok()?;
    (previous.len() <= MAX_DIFF_BYTES && !previous.contains('\0')).then_some(Some(previous))
}

pub(super) fn existing_path(root: &Path, value: &str) -> Result<PathBuf, String> {
    if crate::output::is_reference(value) {
        return Err("Saved tool output is read-only".into());
    }
    let path = fs::canonicalize(root.join(value))
        .map_err(|error| format!("Could not resolve path {value:?}: {error}"))?;
    check_path(root, path)
}

pub(super) fn writable_path(root: &Path, value: &str) -> Result<PathBuf, String> {
    if crate::output::is_reference(value) {
        return Err("Saved tool output is read-only".into());
    }
    let path = root.join(value);
    match fs::symlink_metadata(&path) {
        Ok(_) => existing_path(root, value),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let name = path.file_name().ok_or("The path must name a file")?;
            let parent = path
                .parent()
                .ok_or("The path must have a parent directory")?;
            let parent = fs::canonicalize(parent)
                .map_err(|error| format!("The parent directory must exist: {error}"))?;
            check_path(root, parent.join(name))
        }
        Err(error) => Err(format!("Could not inspect path: {error}")),
    }
}

fn check_path(root: &Path, path: PathBuf) -> Result<PathBuf, String> {
    if !path.starts_with(root) {
        return Err("File tools are restricted to the working directory".into());
    }
    Ok(path)
}

fn read_text(path: &Path, cancellation: &Cancellation) -> Result<String, String> {
    if !fs::metadata(path)
        .map_err(|error| error.to_string())?
        .is_file()
    {
        return Err("The path must refer to a regular file".into());
    }
    let mut file = fs::File::open(path).map_err(|error| format!("Could not open file: {error}"))?;
    let mut bytes = Vec::new();
    let mut block = [0; 64 * 1024];
    loop {
        check_cancelled(cancellation)?;
        let count = file
            .read(&mut block)
            .map_err(|error| format!("Could not read file: {error}"))?;
        if count == 0 {
            break;
        }
        bytes.extend_from_slice(&block[..count]);
    }
    let text = String::from_utf8(bytes).map_err(|_| "File is not UTF-8 text".to_string())?;
    if text.contains('\0') {
        return Err("File contains binary data".into());
    }
    Ok(text)
}

fn replace_file(path: &Path, content: &str, cancellation: &Cancellation) -> Result<(), String> {
    check_cancelled(cancellation)?;
    if content.contains('\0') {
        return Err("File content must be UTF-8 text without NUL bytes".into());
    }
    let metadata = match fs::metadata(path) {
        Ok(metadata) if metadata.is_file() => Some(metadata),
        Ok(_) => return Err("The path must refer to a regular file".into()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(format!("Could not inspect file: {error}")),
    };
    if metadata
        .as_ref()
        .is_some_and(|metadata| metadata.permissions().readonly())
    {
        return Err("File is read-only; its content was not changed".into());
    }
    let temporary = crate::disk::temporary_beside(path);
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .map_err(|error| format!("Could not create temporary file: {error}"))?;
    let result = (|| -> Result<(), String> {
        if let Some(metadata) = metadata {
            file.set_permissions(metadata.permissions())
                .map_err(|error| error.to_string())?;
        }
        for block in content.as_bytes().chunks(64 * 1024) {
            check_cancelled(cancellation)?;
            file.write_all(block).map_err(|error| error.to_string())?;
        }
        check_cancelled(cancellation)?;
        file.sync_all().map_err(|error| error.to_string())?;
        drop(file);
        check_cancelled(cancellation)?;
        fs::rename(&temporary, path).map_err(|error| error.to_string())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result.map_err(|error| format!("Could not replace file: {error}"))
}

fn check_cancelled(cancellation: &Cancellation) -> Result<(), String> {
    if cancellation.requested() {
        Err("Operation cancelled".into())
    } else {
        Ok(())
    }
}

fn check_guard(guard: Option<&FileGuard<'_>>, cancellation: &Cancellation) -> Result<(), String> {
    check_cancelled(cancellation)?;
    if let Some(guard) = guard {
        guard.check_cancelled()?;
    }
    Ok(())
}

#[cfg(test)]
mod batch_tests;
#[cfg(test)]
mod tests;
