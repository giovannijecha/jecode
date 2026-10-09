use super::required_string;
use crate::json::Value;
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_FILE: AtomicU64 = AtomicU64::new(0);

mod mismatch;
mod page;
pub(super) use page::read_page;
pub(crate) use page::read_text_page;

pub(super) fn read(
    root: &Path,
    arguments: &Value,
    cancellation: &crate::cancel::Cancellation,
) -> Result<Value, String> {
    let path = existing_path(root, required_string(arguments, "path")?)?;
    read_page(&path, arguments, cancellation)
}

pub(super) fn write(root: &Path, arguments: &Value) -> Result<Value, String> {
    let path = writable_path(root, required_string(arguments, "path")?)?;
    let content = required_string(arguments, "content")?;
    replace_file(&path, content)?;
    Ok(Value::object([(
        "bytes_written",
        Value::number(content.len()),
    )]))
}

pub(super) fn edit(root: &Path, arguments: &Value) -> Result<Value, String> {
    let path = existing_path(root, required_string(arguments, "path")?)?;
    let old = required_string(arguments, "old_text")?;
    let new = required_string(arguments, "new_text")?;
    if old.is_empty() {
        return Err("old_text must not be empty; use write to replace the whole file".into());
    }
    let content = read_text(&path)?;
    let Some(position) = content.find(old) else {
        return Ok(mismatch::result(
            &content,
            "old_text was not found; use exact current text from the supplied file context or read the file",
        ));
    };
    // Include overlapping matches, which are ambiguous too.
    let next = position + content[position..].chars().next().unwrap().len_utf8();
    if content[next..].contains(old) {
        return Ok(mismatch::result(
            &content,
            "old_text occurs more than once; include more exact surrounding text from the supplied file context or read the file",
        ));
    }
    let mut updated = content;
    updated.replace_range(position..position + old.len(), new);
    replace_file(&path, &updated)?;
    Ok(Value::object([(
        "bytes_written",
        Value::number(updated.len()),
    )]))
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
