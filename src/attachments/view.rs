//! Provider-independent attachment descriptions and request size estimates.

use super::{Attachment, Kind, of_message};
use crate::json::Value;

/// Selected model input capabilities; `None` means not yet known.
#[derive(Clone, Copy, Debug, Default)]
pub struct Inputs {
    pub image: Option<bool>,
    pub file: Option<bool>,
}

/// Plain-text manifest for history reads and compaction summaries.
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

/// Text of a stored user message, including reachable local references.
pub fn user_text(message: &Value) -> String {
    let text = message.get("content").and_then(Value::as_str).unwrap_or("");
    let attachments = of_message(message);
    if attachments.is_empty() {
        text.to_owned()
    } else {
        format!("{text}\n\nAttachments:\n{}", manifest(&attachments))
    }
}

/// A tool result can ask the next request to include a local image or PDF.
pub fn requested(message: &Value) -> Option<Attachment> {
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

/// Tool views after the last assistant message still belong to the next request.
/// A later user message does not consume them before the model has seen them.
pub fn active_view_start(messages: &[Value]) -> usize {
    messages
        .iter()
        .rposition(|message| message.get("role").and_then(Value::as_str) == Some("assistant"))
        .map_or(0, |index| index + 1)
}

/// Estimated token cost of attachment views added to the next request.
#[cfg(test)]
pub fn weight(message: &Value) -> usize {
    weight_for(message, Inputs::default())
}

pub fn weight_for(message: &Value, inputs: Inputs) -> usize {
    let attachments = match message.get("role").and_then(Value::as_str) {
        Some("user") => of_message(message),
        Some("tool") => requested(message).into_iter().collect(),
        _ => return 0,
    };
    attachments
        .iter()
        .filter(|attachment| attachment.kind() != Kind::Image || inputs.image != Some(false))
        .map(tokens)
        .sum()
}

fn tokens(attachment: &Attachment) -> usize {
    match attachment.kind() {
        Kind::Image => match (attachment.width, attachment.height) {
            (Some(width), Some(height)) => {
                let scale = (2048.0 / f64::from(width.max(height))).min(1.0);
                let pixels = f64::from(width) * scale * f64::from(height) * scale;
                ((pixels / 750.0) as usize).clamp(85, 1600)
            }
            _ => 1600,
        },
        Kind::Pdf if attachment.size <= super::PDF_INLINE_BYTES => attachment
            .pages
            .map_or((attachment.size / 64) as usize, |pages| {
                pages as usize * 1500
            }),
        _ => 0,
    }
}
