//! Multimodal `read` results for local files and `attachment:<id>` references.

use crate::{
    attachments::{Kind, Pool, media},
    cancel::Cancellation,
    json::Value,
};
use std::fs::{self, File};
use std::io::Read;
use std::path::Path;

pub fn is_reference(path: &str) -> bool {
    path.starts_with("attachment:")
}

pub(super) fn read_file(
    pool: Option<&Pool>,
    root: &Path,
    arguments: &Value,
    cancellation: &Cancellation,
) -> Result<Value, String> {
    let path = super::files::existing_path(root, super::required_string(arguments, "path")?)?;
    if cancellation.requested() {
        return Err("Operation cancelled".into());
    }
    let metadata = fs::metadata(&path).map_err(|error| error.to_string())?;
    if !metadata.is_file() {
        return Err("The path must refer to a regular file".into());
    }
    // Signatures fit in this prefix. The pool inspects the imported copy for
    // full dimensions and PDF metadata only when a visual file is encountered.
    let mut head = Vec::with_capacity(32);
    File::open(&path)
        .and_then(|file| file.take(32).read_to_end(&mut head))
        .map_err(|error| format!("Could not read file: {error}"))?;
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    let detected = media::detect(&head, &name, metadata.len() <= head.len() as u64);
    if matches!(media::kind(&detected.media), Kind::Image | Kind::Pdf) {
        let pool = pool.ok_or("Image and PDF reads require session attachment storage")?;
        let attachment = pool.import_file(&path, cancellation)?;
        read(Some(pool), &attachment.reference(), arguments, cancellation)
    } else {
        super::files::read_page(&path, arguments, cancellation)
    }
}

pub(super) fn read(
    pool: Option<&Pool>,
    path: &str,
    arguments: &Value,
    cancellation: &Cancellation,
) -> Result<Value, String> {
    let pool = pool.ok_or("Attachment storage is unavailable in this session")?;
    let id = path.trim_start_matches("attachment:");
    let stored = pool.load(id)?;
    let attachment = &stored.attachment;
    let (mut result, note) = match attachment.kind() {
        Kind::Text => (
            super::files::read_page(&stored.path, arguments, cancellation)?,
            None,
        ),
        Kind::Image => (
            Value::object([("view", Value::string("image"))]),
            Some("The image follows in the next user message."),
        ),
        Kind::Pdf => (
            Value::object([("view", Value::string("pdf"))]),
            Some("The PDF follows in the next user message as a file part."),
        ),
        Kind::Binary => (
            Value::object([]),
            Some(
                "Jecode has not interpreted this file. Inspect the original at local_path with tools suited to its format.",
            ),
        ),
    };
    if let Value::Object(fields) = &mut result {
        fields.insert("attachment".into(), attachment.value());
        fields.insert("reference".into(), Value::string(attachment.reference()));
        fields.insert(
            "local_path".into(),
            Value::string(stored.path.display().to_string()),
        );
        if let Some(source) = &stored.source {
            fields.insert("attached_from".into(), Value::string(source));
        }
        if let Some(note) = note {
            fields.insert("note".into(), Value::string(note));
        }
    }
    Ok(result)
}
