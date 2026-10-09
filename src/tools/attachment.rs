//! `read` for `attachment:<id>` references: the session's attached files,
//! including ones that live outside the working directory.

use crate::{
    attachments::{Kind, Pool},
    cancel::Cancellation,
    json::Value,
};

pub fn is_reference(path: &str) -> bool {
    path.starts_with("attachment:")
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
