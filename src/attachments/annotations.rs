//! Keep provider PDF annotation data URLs in the attachment pool.
//!
//! The stored assistant message retains the parser hash and text, but uses
//! attachment references for binary content. Request materialization restores
//! those URLs so OpenRouter can reuse the parsed result on later turns.

use super::{Pool, base64};
use crate::json::Value;
use std::collections::BTreeSet;

const REFERENCE: &str = "jecode_attachment";

pub fn references(message: &Value) -> BTreeSet<String> {
    let mut found = BTreeSet::new();
    if let Some(annotations) = message.get("annotations") {
        collect(annotations, &mut found);
    }
    found
}

fn collect(value: &Value, found: &mut BTreeSet<String>) {
    match value {
        Value::Object(fields) => {
            if let Some(id) = fields
                .get(REFERENCE)
                .and_then(Value::as_str)
                .and_then(|reference| reference.strip_prefix("attachment:"))
                && super::valid_id(id)
            {
                found.insert(id.to_owned());
            }
            for value in fields.values() {
                collect(value, found);
            }
        }
        Value::Array(items) => {
            for item in items {
                collect(item, found);
            }
        }
        _ => {}
    }
}

pub fn store(message: &mut Value, pool: Option<&Pool>) -> Result<(), String> {
    let Value::Object(fields) = message else {
        return Ok(());
    };
    let Some(annotations) = fields.get_mut("annotations") else {
        return Ok(());
    };
    // Leave the original intact if importing any asset fails. The caller can
    // then discard annotations while preserving the assistant's answer.
    let mut stored = annotations.clone();
    store_value(&mut stored, pool)?;
    *annotations = stored;
    Ok(())
}

fn store_value(value: &mut Value, pool: Option<&Pool>) -> Result<(), String> {
    match value {
        Value::String(url) => {
            if let Some((media, data)) = data_url(url) {
                let pool = pool.ok_or("PDF annotation storage is unavailable")?;
                let bytes = base64::decode(data)?;
                let name = match media.split(';').next().unwrap_or("") {
                    "image/png" => "pdf-page.png",
                    "image/jpeg" => "pdf-page.jpg",
                    "image/webp" => "pdf-page.webp",
                    "image/gif" => "pdf-page.gif",
                    _ => "pdf-annotation.bin",
                };
                let asset = pool.import_bytes(name, &bytes)?;
                *value = Value::object([
                    (REFERENCE, Value::string(asset.reference())),
                    ("media", Value::string(media)),
                ]);
            }
        }
        Value::Array(items) => {
            for item in items {
                store_value(item, pool)?;
            }
        }
        Value::Object(fields) => {
            for value in fields.values_mut() {
                store_value(value, pool)?;
            }
        }
        _ => {}
    }
    Ok(())
}

pub fn restore(message: &mut Value, pool: Option<&Pool>) -> Result<(), String> {
    let Value::Object(fields) = message else {
        return Ok(());
    };
    if let Some(annotations) = fields.get_mut("annotations") {
        restore_value(annotations, pool)?;
    }
    Ok(())
}

fn restore_value(value: &mut Value, pool: Option<&Pool>) -> Result<(), String> {
    match value {
        Value::Object(fields) => {
            if let Some(reference) = fields.get(REFERENCE).and_then(Value::as_str) {
                let id = reference
                    .strip_prefix("attachment:")
                    .filter(|id| super::valid_id(id))
                    .ok_or("Invalid PDF annotation reference")?;
                let media = fields
                    .get("media")
                    .and_then(Value::as_str)
                    .ok_or("PDF annotation media type is missing")?;
                let pool = pool.ok_or("PDF annotation storage is unavailable")?;
                let stored = pool.load(id)?;
                let bytes = std::fs::read(&stored.path)
                    .map_err(|error| format!("Cannot read PDF annotation {id}: {error}"))?;
                *value = Value::string(format!("data:{media};base64,{}", base64::encode(&bytes)));
            } else {
                for value in fields.values_mut() {
                    restore_value(value, pool)?;
                }
            }
        }
        Value::Array(items) => {
            for item in items {
                restore_value(item, pool)?;
            }
        }
        _ => {}
    }
    Ok(())
}

/// Historical sessions may already contain inline annotation images. Hide
/// those bytes from the history read tool without changing provider replay.
pub fn history(message: &mut Value) {
    if let Value::Object(fields) = message
        && let Some(annotations) = fields.get_mut("annotations")
    {
        history_value(annotations);
    }
}

fn history_value(value: &mut Value) {
    match value {
        Value::String(url) if data_url(url).is_some() => {
            *value = Value::string("[inline PDF image omitted]");
        }
        Value::Array(items) => {
            for item in items {
                history_value(item);
            }
        }
        Value::Object(fields) => {
            for value in fields.values_mut() {
                history_value(value);
            }
        }
        _ => {}
    }
}

fn data_url(url: &str) -> Option<(&str, &str)> {
    url.strip_prefix("data:")?.split_once(";base64,")
}
