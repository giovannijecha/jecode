use super::{
    Document, Store, Summary,
    document::{integer, string},
    identifier, same_directory,
    storage::{options, reject_special},
};
use crate::json::{self, Value};
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::PathBuf;
use std::time::UNIX_EPOCH;

const LIMIT: u64 = 64 * 1024;

#[derive(PartialEq)]
pub(super) struct Stamp {
    bytes: u64,
    modified: String,
}

pub(super) fn stamp(store: &Store, id: &str) -> Option<Stamp> {
    let path = store.journal_path(id);
    reject_special(&path).ok()?;
    let metadata = fs::metadata(path).ok()?;
    Some(Stamp {
        bytes: metadata.len(),
        modified: metadata
            .modified()
            .ok()?
            .duration_since(UNIX_EPOCH)
            .ok()?
            .as_nanos()
            .to_string(),
    })
}

fn path(store: &Store, id: &str) -> PathBuf {
    store.bucket.join(format!("{id}.summary.json"))
}

pub(super) fn load(store: &Store, id: &str, stamp: &Stamp) -> Option<Summary> {
    let path = path(store, id);
    reject_special(&path).ok()?;
    let mut text = String::new();
    File::open(path)
        .ok()?
        .take(LIMIT + 1)
        .read_to_string(&mut text)
        .ok()?;
    if text.len() as u64 > LIMIT {
        return None;
    }
    let value = json::parse(&text).ok()?;
    if value.get("format").and_then(Value::as_str) != Some("jecode.session.summary")
        || value.get("format_version") != Some(&Value::number(1))
        || string(&value, "id").ok()? != id
        || integer(&value, "journal_bytes").ok()? != stamp.bytes
        || string(&value, "journal_modified").ok()? != stamp.modified
        || !same_directory(
            &PathBuf::from(string(&value, "working_directory").ok()?),
            &store.directory,
        )
    {
        return None;
    }
    let title = string(&value, "title").ok()?;
    let model = string(&value, "model").ok()?;
    if title.chars().count() > 100 || crate::openrouter::validate_model(model).is_err() {
        return None;
    }
    Some(Summary {
        id: id.into(),
        title: title.into(),
        model: model.into(),
        updated: integer(&value, "updated_at_unix_ms").ok()?,
    })
}

pub(super) fn save(store: &Store, document: &Document, observed: &Stamp) -> Result<(), String> {
    if stamp(store, &document.id).as_ref() != Some(observed) {
        return Ok(());
    }
    let value = Value::object([
        ("format", Value::string("jecode.session.summary")),
        ("format_version", Value::number(1)),
        ("id", Value::string(&document.id)),
        (
            "working_directory",
            Value::string(document.directory.to_string_lossy()),
        ),
        ("title", Value::string(document.title())),
        ("model", Value::string(&document.model)),
        ("updated_at_unix_ms", Value::number(document.updated)),
        ("journal_bytes", Value::number(observed.bytes)),
        ("journal_modified", Value::string(&observed.modified)),
    ]);
    let bytes = value.encode();
    if bytes.len() as u64 > LIMIT {
        return Ok(());
    }
    let path = path(store, &document.id);
    reject_special(&path)?;
    let temporary = path.with_extension(format!("tmp-{}", identifier()));
    let mut file = options()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .map_err(|error| error.to_string())?;
    let written = file
        .write_all(bytes.as_bytes())
        .and_then(|_| file.sync_all());
    drop(file);
    let result = written.and_then(|_| fs::rename(&temporary, path));
    if result.is_err() {
        let _ = fs::remove_file(temporary);
    }
    result.map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests;
