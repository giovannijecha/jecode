//! Turns stored attachment references into OpenRouter request content.
//!
//! Conversation history keeps user content as text plus attachment metadata.
//! Right before a request, `materialize` reads the session-owned assets and
//! builds multipart content: images as `image_url` parts, PDFs as `file`
//! parts, and a manifest that tells the model what each element is and how to
//! reach it again. Nothing here changes the stored history.

use super::{Attachment, Kind, Pool, base64};
use crate::json::Value;

/// PDFs above this size stay local; the manifest points at them instead.
const PDF_BYTES: u64 = 32 * 1024 * 1024;
/// Estimated tokens for an image whose dimensions are unknown.
const IMAGE_TOKENS: usize = 1600;

/// What the selected model accepts, from the model catalog. `None` is unknown.
#[derive(Clone, Copy, Debug, Default)]
pub struct Inputs {
    pub image: Option<bool>,
    pub file: Option<bool>,
}

/// The OpenRouter PDF engine. Models with native file input read the PDF
/// themselves; the others use the free Cloudflare parser instead of the
/// paid OCR default.
pub fn engine(file_input: Option<bool>) -> &'static str {
    if file_input == Some(true) {
        "native"
    } else {
        "cloudflare-ai"
    }
}

pub fn plugins(file_input: Option<bool>) -> Value {
    Value::Array(vec![Value::object([
        ("id", Value::string("file-parser")),
        (
            "pdf",
            Value::object([("engine", Value::string(engine(file_input)))]),
        ),
    ])])
}

/// Whether a request carries a file part, and so needs the parser plugin.
pub fn has_files(messages: &[Value]) -> bool {
    messages.iter().any(|message| {
        message
            .get("content")
            .and_then(Value::as_array)
            .is_some_and(|parts| {
                parts
                    .iter()
                    .any(|part| part.get("type").and_then(Value::as_str) == Some("file"))
            })
    })
}

/// Builds request messages: user attachments become content parts and each
/// image or PDF a read tool asked to show follows its tool results as a user part.
pub fn materialize(
    messages: Vec<Value>,
    pool: Option<&Pool>,
    inputs: Inputs,
) -> Result<Vec<Value>, String> {
    let mut result = Vec::with_capacity(messages.len());
    let mut views = Vec::new();
    for message in messages {
        let role = message.get("role").and_then(Value::as_str).unwrap_or("");
        if role != "tool" && !views.is_empty() {
            result.push(shown(std::mem::take(&mut views), pool, inputs));
        }
        if role == "tool" {
            if let Some(attachment) = requested(&message) {
                views.push(attachment);
            }
            result.push(message);
        } else if role == "user" && message.get("attachments").is_some() {
            result.push(user(message, pool, inputs));
        } else {
            let mut message = message;
            super::annotations::restore(&mut message, pool)?;
            result.push(message);
        }
    }
    if !views.is_empty() {
        result.push(shown(views, pool, inputs));
    }
    Ok(result)
}

fn user(message: Value, pool: Option<&Pool>, inputs: Inputs) -> Value {
    let attachments = super::of_message(&message);
    let Value::Object(mut fields) = message else {
        return message;
    };
    fields.remove("attachments");
    let text = fields
        .get("content")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned();
    let mut parts = Vec::new();
    let mut lines = Vec::new();
    for (index, attachment) in attachments.iter().enumerate() {
        let (part, status) = content(attachment, pool, inputs);
        lines.push(line(index + 1, attachment, pool, &status));
        parts.extend(part);
    }
    let manifest = format!("Attachments:\n{}", lines.join("\n"));
    let text = if text.trim().is_empty() {
        manifest
    } else {
        format!("{text}\n\n{manifest}")
    };
    parts.insert(0, text_part(&text));
    fields.insert("content".into(), Value::Array(parts));
    Value::Object(fields)
}

/// The content part for one attachment and a short status for the manifest.
fn content(
    attachment: &Attachment,
    pool: Option<&Pool>,
    inputs: Inputs,
) -> (Option<Value>, String) {
    let Some(pool) = pool else {
        return (None, "not sent: attachment storage is unavailable".into());
    };
    match attachment.kind() {
        Kind::Image if inputs.image == Some(false) => (
            None,
            "not sent: the selected model does not accept image input".into(),
        ),
        Kind::Image => match pool.view(attachment) {
            Ok(Some((media, bytes))) => (Some(image_part(&media, &bytes)), "image included".into()),
            Ok(None) => (
                None,
                "not sent: no provider-compatible image form exists on this system".into(),
            ),
            Err(error) => (None, format!("not sent: {error}")),
        },
        Kind::Pdf if attachment.size > PDF_BYTES => (
            None,
            format!(
                "not sent: larger than {}; the original stays local",
                super::size(PDF_BYTES)
            ),
        ),
        Kind::Pdf => match pool
            .load(&attachment.id)
            .and_then(|stored| std::fs::read(&stored.path).map_err(|error| error.to_string()))
        {
            Ok(bytes) => (
                Some(file_part(&attachment.name, &bytes)),
                format!(
                    "PDF included; OpenRouter file-parser engine: {}",
                    engine(inputs.file)
                ),
            ),
            Err(error) => (None, format!("not sent: {error}")),
        },
        Kind::Text => (
            None,
            format!(
                "text not inlined; read it with the read tool, path {}",
                attachment.reference()
            ),
        ),
        Kind::Binary => (
            None,
            "not interpreted; only the original bytes are available locally".into(),
        ),
    }
}

fn line(number: usize, attachment: &Attachment, pool: Option<&Pool>, status: &str) -> String {
    let location = pool
        .and_then(|pool| pool.load(&attachment.id).ok())
        .map(|stored| format!("; local copy: {}", stored.path.display()))
        .unwrap_or_default();
    format!(
        "{} {} ({}); reference {}{location}; {status}",
        attachment.label(number),
        attachment.name,
        attachment.details(),
        attachment.reference()
    )
}

/// A user message as text, with a manifest of its attachments so their
/// references stay reachable from history and after compaction.
pub fn user_text(message: &Value) -> String {
    let text = message.get("content").and_then(Value::as_str).unwrap_or("");
    let attachments = super::of_message(message);
    if attachments.is_empty() {
        text.to_owned()
    } else {
        format!("{text}\n\nAttachments:\n{}", manifest(&attachments))
    }
}

/// A plain-text manifest for views that cannot carry content parts, such as
/// history reads and the request list kept after compaction.
pub fn manifest(attachments: &[Attachment]) -> String {
    attachments
        .iter()
        .enumerate()
        .map(|(index, attachment)| {
            format!(
                "{} {} ({}): {}",
                attachment.label(index + 1),
                attachment.name,
                attachment.details(),
                attachment.reference()
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn requested(message: &Value) -> Option<Attachment> {
    let content = message.get("content").and_then(Value::as_str)?;
    if !content.contains("\"view\"") {
        return None;
    }
    let value = crate::json::parse(content).ok()?;
    if !matches!(
        value.get("view").and_then(Value::as_str),
        Some("image" | "pdf")
    ) {
        return None;
    }
    Attachment::parse(value.get("attachment")?).ok()
}

/// A user message with the images and PDFs that read results asked to show.
fn shown(attachments: Vec<Attachment>, pool: Option<&Pool>, inputs: Inputs) -> Value {
    let mut parts = Vec::new();
    let mut notes = Vec::new();
    for attachment in &attachments {
        match content(attachment, pool, inputs) {
            (Some(part), _) => {
                notes.push(format!("{} follows.", attachment.reference()));
                parts.push(part);
            }
            (None, status) => notes.push(format!("{}: {status}", attachment.reference())),
        }
    }
    parts.insert(
        0,
        text_part(&format!(
            "Attachments requested by the read tool:\n{}",
            notes.join("\n")
        )),
    );
    Value::object([
        ("role", Value::string("user")),
        ("content", Value::Array(parts)),
    ])
}

fn text_part(text: &str) -> Value {
    Value::object([
        ("type", Value::string("text")),
        ("text", Value::string(text)),
    ])
}

fn image_part(media: &str, bytes: &[u8]) -> Value {
    Value::object([
        ("type", Value::string("image_url")),
        (
            "image_url",
            Value::object([(
                "url",
                Value::string(format!("data:{media};base64,{}", base64::encode(bytes))),
            )]),
        ),
    ])
}

fn file_part(name: &str, bytes: &[u8]) -> Value {
    Value::object([
        ("type", Value::string("file")),
        (
            "file",
            Value::object([
                ("filename", Value::string(name)),
                (
                    "file_data",
                    Value::string(format!(
                        "data:application/pdf;base64,{}",
                        base64::encode(bytes)
                    )),
                ),
            ]),
        ),
    ])
}

/// Estimated tokens the materialized form adds beyond the stored message.
/// Payload size says nothing useful here: a 4 MB PNG and a 40 KB JPEG of the
/// same dimensions cost the same, so images are weighed by pixels.
pub fn weight(message: &Value) -> usize {
    let attachments = match message.get("role").and_then(Value::as_str) {
        Some("user") => super::of_message(message),
        Some("tool") => requested(message).into_iter().collect(),
        _ => return 0,
    };
    attachments.iter().map(tokens).sum()
}

fn tokens(attachment: &Attachment) -> usize {
    match attachment.kind() {
        Kind::Image => match (attachment.width, attachment.height) {
            (Some(width), Some(height)) => {
                // Views are bounded to 2048 pixels on the long side.
                let scale = (2048.0 / f64::from(width.max(height))).min(1.0);
                let pixels = f64::from(width) * scale * f64::from(height) * scale;
                ((pixels / 750.0) as usize).clamp(85, IMAGE_TOKENS)
            }
            _ => IMAGE_TOKENS,
        },
        Kind::Pdf if attachment.size <= PDF_BYTES => attachment
            .pages
            .map_or((attachment.size / 64) as usize, |pages| {
                pages as usize * 1500
            }),
        _ => 0,
    }
}

#[cfg(test)]
mod tests;
