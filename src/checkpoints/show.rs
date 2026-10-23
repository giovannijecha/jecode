//! Read a call's saved file without changing the workspace.
use super::Store;
use super::paths;
use super::state::State;
use super::store::Loaded;
use crate::cancel::Cancellation;
use crate::json::Value;
use std::io::{BufReader, Read, Write};
use std::sync::Arc;

pub const PREFIX: &str = "checkpoint:";

pub fn is_reference(path: &str) -> bool {
    path.starts_with(PREFIX)
}

fn resolve(
    store: &Store,
    reference: &str,
    cancel: &Cancellation,
) -> Result<(Loaded, String, State), String> {
    let rest = reference
        .strip_prefix(PREFIX)
        .ok_or("Not a checkpoint reference")?;
    let (reference, path) = rest
        .split_once(':')
        .ok_or("Use checkpoint:rc-ID:workspace-relative-path")?;
    let loaded = store.load(reference, cancel)?;
    let path = paths::resolve_saved(&store.root, &loaded.snapshot, path)?;
    let state = loaded
        .snapshot
        .entries
        .get(&path)
        .cloned()
        .ok_or_else(|| format!("{path} was not saved by this call"))?;
    Ok((loaded, path, state))
}

pub fn read(
    store: &Arc<Store>,
    reference: &str,
    args: &Value,
    cancel: &Cancellation,
) -> Result<Value, String> {
    let (loaded, path, state) = resolve(store, reference, cancel)?;
    match &state {
        State::File { length, .. } => {
            loaded.verify(&state, cancel)?;
            crate::tools::read_source(
                BufReader::new(loaded.reader(&state)?),
                *length,
                args,
                cancel,
            )
        }
        State::Directory { .. } => {
            let prefix = format!("{path}/");
            let entries = loaded
                .snapshot
                .entries
                .iter()
                .filter_map(|(name, state)| {
                    let child = name.strip_prefix(&prefix)?;
                    (!child.contains('/') && state.exists()).then(|| Value::string(child))
                })
                .collect();
            Ok(Value::object([("entries", Value::Array(entries))]))
        }
        State::Link { target, .. } => Ok(Value::object([("link", Value::string(target))])),
        State::Absent => Err(format!("{path} did not exist before this call")),
        State::Unreadable { reason } => Err(format!("{path} was not captured: {reason}")),
    }
}

/// CLI output is streamed, with integrity checked before emitting saved bytes.
pub fn write_text(
    store: &Arc<Store>,
    reference: &str,
    output: &mut impl Write,
) -> Result<(), String> {
    let cancel = Cancellation::default();
    let (loaded, path, state) = resolve(store, reference, &cancel)?;
    if !matches!(state, State::File { .. }) {
        return Err(format!("{path} is not a saved regular file"));
    }
    loaded.verify(&state, &cancel)?;
    let mut reader = loaded.reader(&state)?;
    if !crate::utf8::is_text(loaded.reader(&state)?).map_err(|error| error.to_string())? {
        return Err("Saved file contains binary data; restore it to use its content".into());
    }
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let count = reader
            .read(&mut buffer)
            .map_err(|error| error.to_string())?;
        if count == 0 {
            break;
        }
        output
            .write_all(&buffer[..count])
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}
