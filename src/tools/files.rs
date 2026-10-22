use super::required_string;
use crate::checkpoints::session::{FileGuard, Session};
use crate::{file_change::MAX_DIFF_BYTES, json::Value};
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_FILE: AtomicU64 = AtomicU64::new(0);

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
    write_with_change(root, arguments, None).map(|(result, _)| result)
}

pub(super) fn write_with_change(
    root: &Path,
    arguments: &Value,
    checkpoints: Option<&Session>,
) -> Result<(Value, Option<crate::file_change::Change>), String> {
    let path = writable_path(root, required_string(arguments, "path")?)?;
    let content = required_string(arguments, "content")?;
    let mut guard = checkpoints.map(|session| session.file("write", &path));
    // A write may replace non-text data; inability to read it cannot reject
    // an operation that was previously allowed.
    let previous = guard.as_mut().map_or_else(
        || previous_text(&path, content.len()),
        |guard| {
            guard
                .prepare_with_previous(content.as_bytes(), MAX_DIFF_BYTES)
                .or_else(|| previous_text(&path, content.len()))
        },
    );
    let checkpoint = finish_replacement(guard, &path, content)?;
    let change =
        previous.and_then(|old| crate::file_change::Change::between(old.as_deref(), content));
    Ok((written(content.len() as u64, checkpoint), change))
}

#[cfg(test)]
pub(super) fn edit(root: &Path, arguments: &Value) -> Result<Value, String> {
    edit_with_change(root, arguments, None).map(|(result, _)| result)
}

pub(super) fn edit_with_change(
    root: &Path,
    arguments: &Value,
    checkpoints: Option<&Session>,
) -> Result<(Value, Option<crate::file_change::Change>), String> {
    let path = existing_path(root, required_string(arguments, "path")?)?;
    let old = required_string(arguments, "old_text")?;
    let new = required_string(arguments, "new_text")?;
    if old.is_empty() {
        return Err("old_text must not be empty; use write to replace the whole file".into());
    }
    // Hold the path from reading to replacing so concurrent sessions take turns.
    let guard = checkpoints.map(|session| session.file("edit", &path));
    if fs::metadata(&path).is_ok_and(|metadata| metadata.len() > MAX_DIFF_BYTES as u64) {
        return streaming::edit(&path, old, new, guard);
    }
    let mut content = read_text(&path)?;
    let Some(position) = content.find(old) else {
        return Ok((
            mismatch::result(
                &content,
                "old_text was not found; use exact current text from the supplied file context or read the file",
            ),
            None,
        ));
    };
    // Include overlapping matches, which are ambiguous too.
    let next = position + content[position..].chars().next().unwrap().len_utf8();
    if content[next..].contains(old) {
        return Ok((
            mismatch::result(
                &content,
                "old_text occurs more than once; include more exact surrounding text from the supplied file context or read the file",
            ),
            None,
        ));
    }
    let updated_len = content.len() - old.len() + new.len();
    let include_change = content.len() <= MAX_DIFF_BYTES && updated_len <= MAX_DIFF_BYTES;
    let mut updated = if include_change {
        content.clone()
    } else {
        std::mem::take(&mut content)
    };
    updated.replace_range(position..position + old.len(), new);
    let checkpoint = replace_protected(guard, &path, &updated)?;
    let change = include_change
        .then(|| crate::file_change::Change::between(Some(&content), &updated))
        .flatten();
    Ok((written(updated.len() as u64, checkpoint), change))
}

/// Saves the previous state before replacing the file. The replacement
/// proceeds whether or not saving succeeded; the result says which.
fn replace_protected(
    guard: Option<FileGuard<'_>>,
    path: &Path,
    content: &str,
) -> Result<Option<Value>, String> {
    let Some(mut guard) = guard else {
        return replace_file(path, content).map(|()| None);
    };
    guard.prepare(content.as_bytes());
    finish_replacement(Some(guard), path, content)
}

fn finish_replacement(
    guard: Option<FileGuard<'_>>,
    path: &Path,
    content: &str,
) -> Result<Option<Value>, String> {
    let Some(guard) = guard else {
        return replace_file(path, content).map(|()| None);
    };
    let replaced = replace_file(path, content);
    let checkpoint = guard.finish(replaced.is_ok());
    replaced.map(|()| Some(checkpoint))
}

fn written(bytes: u64, checkpoint: Option<Value>) -> Value {
    let mut result = Value::object([("bytes_written", Value::number(bytes))]);
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

fn read_text(path: &Path) -> Result<String, String> {
    if !fs::metadata(path)
        .map_err(|error| error.to_string())?
        .is_file()
    {
        return Err("The path must refer to a regular file".into());
    }
    let mut file = fs::File::open(path).map_err(|error| format!("Could not open file: {error}"))?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)
        .map_err(|error| format!("Could not read file: {error}"))?;
    let text = String::from_utf8(bytes).map_err(|_| "File is not UTF-8 text".to_string())?;
    if text.contains('\0') {
        return Err("File contains binary data".into());
    }
    Ok(text)
}

fn replace_file(path: &Path, content: &str) -> Result<(), String> {
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
    let temporary = path.with_file_name(format!(
        ".jecode-{}-{}.tmp",
        std::process::id(),
        NEXT_FILE.fetch_add(1, Ordering::Relaxed)
    ));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .map_err(|error| format!("Could not create temporary file: {error}"))?;
    let result = (|| {
        if let Some(metadata) = metadata {
            file.set_permissions(metadata.permissions())?;
        }
        file.write_all(content.as_bytes())?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temporary, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result.map_err(|error| format!("Could not replace file: {error}"))
}

#[cfg(test)]
mod tests;
