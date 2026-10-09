//! Prompt attachments: the structured prompt shared by the composer, sessions,
//! the agent and the provider, plus the session-owned asset pool.
//!
//! A prompt keeps its text and its attachments apart. Each attachment owns one
//! object replacement character in the text; the n-th marker is the n-th
//! attachment. Labels such as `[1# Image]` are derived for display only, so a
//! typed lookalike label is plain text and never an attachment.

pub mod annotations;
pub mod base64;
pub mod capture;
mod media;
pub mod paths;
mod pool;
pub mod provider;

pub use pool::{Pool, references};

use crate::json::Value;

/// Stands in for one attachment inside prompt text.
pub const MARKER: char = '\u{fffc}';

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Image,
    Pdf,
    Text,
    Binary,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Attachment {
    pub id: String,
    pub name: String,
    pub media: String,
    pub size: u64,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub pages: Option<u32>,
}

impl Attachment {
    pub fn kind(&self) -> Kind {
        media::kind(&self.media)
    }

    pub fn label(&self, number: usize) -> String {
        match self.kind() {
            Kind::Image => format!("[{number}# Image]"),
            _ => format!("[{number}# File: {}]", self.name),
        }
    }

    /// A reference the read tool resolves to this attachment.
    pub fn reference(&self) -> String {
        format!("attachment:{}", self.id)
    }

    pub fn details(&self) -> String {
        let mut parts = vec![self.media.clone()];
        if let (Some(width), Some(height)) = (self.width, self.height) {
            parts.push(format!("{width}x{height}"));
        }
        if let Some(pages) = self.pages {
            parts.push(format!("{pages} page{}", if pages == 1 { "" } else { "s" }));
        }
        parts.push(size(self.size));
        parts.join(" · ")
    }

    pub fn value(&self) -> Value {
        let mut fields = std::collections::BTreeMap::new();
        fields.insert("id".into(), Value::string(&self.id));
        fields.insert("name".into(), Value::string(&self.name));
        fields.insert("media".into(), Value::string(&self.media));
        fields.insert("size".into(), Value::number(self.size));
        for (key, value) in [
            ("width", self.width),
            ("height", self.height),
            ("pages", self.pages),
        ] {
            if let Some(value) = value {
                fields.insert(key.into(), Value::number(value));
            }
        }
        Value::Object(fields)
    }

    pub fn parse(value: &Value) -> Result<Self, String> {
        let text = |key: &str| {
            value
                .get(key)
                .and_then(Value::as_str)
                .map(str::to_owned)
                .ok_or_else(|| format!("Attachment {key} must be a string"))
        };
        let number = |key: &str| {
            value
                .get(key)
                .and_then(Value::as_usize)
                .and_then(|value| u32::try_from(value).ok())
        };
        let id = text("id")?;
        if !valid_id(&id) {
            return Err("Attachment id is invalid".into());
        }
        Ok(Self {
            id,
            name: text("name")?,
            media: text("media")?,
            size: value
                .get("size")
                .and_then(Value::as_usize)
                .ok_or("Attachment size must be a number")? as u64,
            width: number("width"),
            height: number("height"),
            pages: number("pages"),
        })
    }
}

pub fn valid_id(id: &str) -> bool {
    id.strip_prefix("att-")
        .is_some_and(|rest| rest.len() <= 76 && crate::sessions::valid_id(rest))
}

pub fn new_id() -> String {
    format!("att-{}", crate::sessions::identifier())
}

/// Text plus the attachments its markers own.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Prompt {
    pub text: String,
    pub attachments: Vec<Attachment>,
}

impl Prompt {
    pub fn plain(text: impl Into<String>) -> Self {
        Self {
            text: strip(&text.into()),
            attachments: Vec::new(),
        }
    }

    /// Builds a prompt and repairs a mismatched marker count instead of
    /// letting a marker point at the wrong attachment.
    pub fn new(text: String, attachments: Vec<Attachment>) -> Self {
        let markers = text.chars().filter(|&c| c == MARKER).count();
        if markers == attachments.len() {
            return Self { text, attachments };
        }
        let mut text = strip(&text);
        if !attachments.is_empty() && !text.is_empty() && !text.ends_with(char::is_whitespace) {
            text.push(' ');
        }
        attachments.iter().for_each(|_| text.push(MARKER));
        Self { text, attachments }
    }

    pub fn is_empty(&self) -> bool {
        self.attachments.is_empty() && self.text.trim().is_empty()
    }

    /// Text with each marker replaced by its numbered label.
    pub fn display(&self) -> String {
        expand(&self.text, &self.attachments)
    }

    pub fn value(&self) -> Value {
        if self.attachments.is_empty() {
            return Value::string(&self.text);
        }
        Value::object([
            ("text", Value::string(&self.text)),
            (
                "attachments",
                Value::Array(self.attachments.iter().map(Attachment::value).collect()),
            ),
        ])
    }

    /// Accepts the object form and the plain strings of older sessions.
    pub fn parse(value: &Value) -> Result<Self, String> {
        if let Some(text) = value.as_str() {
            return Ok(Self::plain(text));
        }
        let text = value
            .get("text")
            .and_then(Value::as_str)
            .ok_or("Prompt text must be a string")?;
        let attachments = value
            .get("attachments")
            .and_then(Value::as_array)
            .unwrap_or_default()
            .iter()
            .map(Attachment::parse)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self::new(text.into(), attachments))
    }

    /// The user message stored in the conversation. Content stays a string
    /// so text-only consumers keep working; attachments ride beside it.
    pub fn message(&self) -> Value {
        let mut message = Value::object([
            ("role", Value::string("user")),
            ("content", Value::string(self.display())),
        ]);
        if let Value::Object(fields) = &mut message
            && !self.attachments.is_empty()
        {
            fields.insert(
                "attachments".into(),
                Value::Array(self.attachments.iter().map(Attachment::value).collect()),
            );
        }
        message
    }
}

impl From<&str> for Prompt {
    fn from(text: &str) -> Self {
        Self::plain(text)
    }
}

impl From<&String> for Prompt {
    fn from(text: &String) -> Self {
        Self::plain(text.as_str())
    }
}

impl From<String> for Prompt {
    fn from(text: String) -> Self {
        Self::plain(text)
    }
}

impl From<&Prompt> for Prompt {
    fn from(prompt: &Prompt) -> Self {
        prompt.clone()
    }
}

// Tests compare text-only prompts with plain strings.
#[cfg(test)]
impl PartialEq<str> for Prompt {
    fn eq(&self, other: &str) -> bool {
        self.attachments.is_empty() && self.text == other
    }
}

#[cfg(test)]
impl PartialEq<&str> for Prompt {
    fn eq(&self, other: &&str) -> bool {
        self == *other
    }
}

#[cfg(test)]
impl PartialEq<String> for Prompt {
    fn eq(&self, other: &String) -> bool {
        self == other.as_str()
    }
}

/// Attachments of a stored user message.
pub fn of_message(message: &Value) -> Vec<Attachment> {
    message
        .get("attachments")
        .and_then(Value::as_array)
        .unwrap_or_default()
        .iter()
        .filter_map(|value| Attachment::parse(value).ok())
        .collect()
}

pub fn expand(text: &str, attachments: &[Attachment]) -> String {
    let mut result = String::with_capacity(text.len());
    let mut number = 0;
    for character in text.chars() {
        if character == MARKER {
            number += 1;
            if let Some(attachment) = attachments.get(number - 1) {
                result.push_str(&attachment.label(number));
            }
        } else {
            result.push(character);
        }
    }
    result
}

/// Removes markers from text that did not come from an attachment.
pub fn strip(text: &str) -> String {
    text.replace(MARKER, "")
}

pub fn size(bytes: u64) -> String {
    match bytes {
        0..1024 => format!("{bytes} B"),
        1024..1_048_576 => format!("{:.1} KiB", bytes as f64 / 1024.0),
        1_048_576..1_073_741_824 => format!("{:.1} MiB", bytes as f64 / 1_048_576.0),
        _ => format!("{:.1} GiB", bytes as f64 / 1_073_741_824.0),
    }
}

#[cfg(test)]
pub(crate) mod tests;
