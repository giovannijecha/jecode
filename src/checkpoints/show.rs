//! Recorded content addressed as `checkpoint:cp-N:path`, read without
//! changing the workspace.
use super::index::{parse_reference, reference};
use super::record::State;
use super::restore::normalize;
use super::workspace::Workspace;
use crate::json::Value;
use std::io::{BufReader, Read, Seek, Write};

pub const PREFIX: &str = "checkpoint:";

pub enum Content {
    Text(String),
    Binary(usize),
    Preview(String, u64),
    Directory(Vec<String>),
    Link(String),
}

pub fn is_reference(path: &str) -> bool {
    path.starts_with(PREFIX)
}

/// The state of a path at a checkpoint.
fn resolve(store: &mut super::store::Store, path: &str) -> Result<(String, u64, State), String> {
    let rest = path
        .strip_prefix(PREFIX)
        .ok_or("Not a checkpoint reference")?;
    let (target, relative) = rest
        .split_once(':')
        .ok_or("Use checkpoint:cp-<number>:<workspace-relative path>")?;
    let target = parse_reference(target)?;
    let relative = normalize(relative)?;
    store.refresh()?;
    let index = &store.index;
    if !index.retained(target) || target > index.latest() {
        return Err(format!(
            "{} is not available; retained history runs from {} to {}",
            reference(target),
            reference(index.horizon),
            reference(index.latest())
        ));
    }
    let state = super::restore::target_state(index, target, &relative)
        .ok_or_else(|| format!("{relative} was not protected by {}", reference(target)))?;
    Ok((relative, target, state))
}

/// The state of a path, with bounded preview for a large text file. Tool reads
/// use `read_page`; plain CLI output uses `write_text` to stream exact bytes.
pub fn content(workspace: &Workspace, path: &str) -> Result<Content, String> {
    let mut store = workspace.store();
    let (relative, target, state) = resolve(&mut store, path)?;
    match state {
        State::File { object, length, .. } => {
            if length > super::capture::INLINE_BYTES
                && !crate::utf8::is_text(store.reader(object)?)
                    .map_err(|error| error.to_string())?
            {
                return Ok(Content::Binary(
                    usize::try_from(length).unwrap_or(usize::MAX),
                ));
            }
            let mut bytes = Vec::new();
            store
                .reader(object)?
                .take(super::capture::INLINE_BYTES)
                .read_to_end(&mut bytes)
                .map_err(|error| error.to_string())?;
            if length > bytes.len() as u64 {
                let valid = match std::str::from_utf8(&bytes) {
                    Ok(_) => bytes.len(),
                    Err(error) if error.error_len().is_none() => error.valid_up_to(),
                    Err(_) => {
                        return Ok(Content::Binary(
                            usize::try_from(length).unwrap_or(usize::MAX),
                        ));
                    }
                };
                if bytes[..valid].contains(&0) {
                    return Ok(Content::Binary(
                        usize::try_from(length).unwrap_or(usize::MAX),
                    ));
                }
                return Ok(Content::Preview(
                    std::str::from_utf8(&bytes[..valid]).unwrap().into(),
                    length,
                ));
            }
            match std::str::from_utf8(&bytes) {
                Ok(text) if !text.contains('\0') => Ok(Content::Text(text.into())),
                _ => Ok(Content::Binary(bytes.len())),
            }
        }
        State::Directory { .. } => {
            let prefix = format!("{relative}/");
            let children = store
                .index
                .paths
                .keys()
                .filter_map(|path| {
                    let child = path.strip_prefix(&prefix)?;
                    let state = super::restore::target_state(&store.index, target, path)?;
                    if !state.exists() {
                        return None;
                    }
                    let directory = matches!(state, State::Directory { .. });
                    (!child.contains('/'))
                        .then(|| format!("{child}{}", if directory { "/" } else { "" }))
                })
                .collect();
            Ok(Content::Directory(children))
        }
        State::Link { target } => Ok(Content::Link(target)),
        State::Absent => Err(format!("{relative} did not exist at {}", reference(target))),
        other => Err(format!(
            "{relative} was not captured at {}: {}",
            reference(target),
            super::actions::describe(&other)
        )),
    }
}

pub fn read_page(
    workspace: &Workspace,
    path: &str,
    arguments: &Value,
    cancellation: &crate::cancel::Cancellation,
) -> Result<Value, String> {
    let mut store = workspace.store();
    let (_, _, state) = resolve(&mut store, path)?;
    if let State::File { object, length, .. } = state {
        let mut reader = store.reader(object)?;
        let mut sample = [0; 4096];
        let count = reader
            .read(&mut sample)
            .map_err(|error| error.to_string())?;
        let binary = sample[..count].contains(&0)
            || std::str::from_utf8(&sample[..count])
                .is_err_and(|error| error.error_len().is_some());
        if binary {
            return Ok(Value::object([
                ("binary", Value::Bool(true)),
                ("bytes", Value::number(length)),
                (
                    "note",
                    Value::string("restore the path to use this content"),
                ),
            ]));
        }
        reader.rewind().map_err(|error| error.to_string())?;
        return crate::tools::read_source(BufReader::new(reader), length, arguments, cancellation);
    }
    drop(store);
    match content(workspace, path)? {
        Content::Directory(entries) => Ok(Value::object([(
            "entries",
            Value::Array(entries.into_iter().map(Value::string).collect()),
        )])),
        Content::Link(target) => Ok(Value::object([("link", Value::string(target))])),
        _ => Err("The checkpoint path changed while being read; retry".into()),
    }
}

/// Returns false for non-text content, allowing the caller's normal rendering.
pub fn write_text(
    workspace: &Workspace,
    path: &str,
    output: &mut impl Write,
) -> Result<bool, String> {
    let mut store = workspace.store();
    let (_, _, state) = resolve(&mut store, path)?;
    let State::File { object, .. } = state else {
        return Ok(false);
    };
    let mut reader = store.reader(object)?;
    if !crate::utf8::is_text(&mut reader).map_err(|error| error.to_string())? {
        return Ok(false);
    }
    reader.rewind().map_err(|error| error.to_string())?;
    std::io::copy(&mut reader, output).map_err(|error| error.to_string())?;
    Ok(true)
}
