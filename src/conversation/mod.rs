//! Stored conversation messages and their provider-owned extensions.
//!
//! The portable shape deliberately keeps the established message keys:
//! `role`, `content`, `tool_calls`, `tool_call_id`, `name`, and `attachments`.
//! Tool calls use flat `id`, `name`, and `arguments` fields. Attachments
//! remain local references beside user content. All other fields returned by
//! a backend are opaque and belong to the backend and model that produced them.
//! Old OpenRouter messages have no envelope; session loading tags a transient
//! request copy with the archived origin. Projection never changes storage.

use crate::json::Value;
use std::collections::BTreeMap;

mod calls;
pub use calls::ToolCall;

const ENVELOPE: &str = "jecode_origin";
const CORE: &[&str] = &[
    "role",
    "content",
    "tool_calls",
    "tool_call_id",
    "name",
    "attachments",
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Origin {
    pub backend: String,
    pub model: String,
    pub account: Option<String>,
}

impl Origin {
    pub fn new(backend: &str, model: &str) -> Self {
        Self {
            backend: backend.into(),
            model: model.into(),
            account: None,
        }
    }

    pub fn with_account(backend: &str, model: &str, account: Option<String>) -> Self {
        Self {
            backend: backend.into(),
            model: model.into(),
            account,
        }
    }

    pub fn value(&self) -> Value {
        let mut value = Value::object([
            ("backend", Value::string(&self.backend)),
            ("model", Value::string(&self.model)),
        ]);
        if let Some(account) = &self.account
            && let Value::Object(fields) = &mut value
        {
            fields.insert("account".into(), Value::string(account));
        }
        value
    }

    pub fn parse(value: &Value) -> Result<Self, String> {
        let field = |key| {
            value
                .get(key)
                .and_then(Value::as_str)
                .filter(|field| !field.trim().is_empty())
                .ok_or_else(|| format!("Conversation origin {key} is invalid"))
        };
        let account = value
            .get("account")
            .map(|value| {
                value
                    .as_str()
                    .filter(|account| !account.is_empty())
                    .map(str::to_owned)
                    .ok_or_else(|| "Conversation origin account is invalid".to_string())
            })
            .transpose()?;
        Ok(Self::with_account(
            field("backend")?,
            field("model")?,
            account,
        ))
    }
}

pub fn has_origin(message: &Value) -> bool {
    message.get(ENVELOPE).is_some()
}

/// Durable accounting stays outside provider extras, so it never reaches the wire.
pub(crate) fn record_output_tokens(message: &mut Value, tokens: Option<usize>) {
    if let Some(tokens) = tokens
        && let Value::Object(fields) = message
        && let Some(Value::Object(origin)) = fields.get_mut(ENVELOPE)
    {
        origin.insert("output_tokens".into(), Value::number(tokens));
    }
}

pub(crate) fn output_tokens(message: &Value) -> Option<usize> {
    message.get(ENVELOPE)?.get("output_tokens")?.as_usize()
}

pub(crate) fn output_tokens_for(message: &Value, current: &Origin) -> Option<usize> {
    (Origin::parse(message.get(ENVELOPE)?).ok().as_ref() == Some(current))
        .then(|| output_tokens(message))
        .flatten()
}

/// A different model or account receives portable text, not the measured native response.
pub(crate) fn retain_output_measurement(message: &mut Value, current: &Origin) {
    if message
        .get(ENVELOPE)
        .and_then(|value| Origin::parse(value).ok())
        .as_ref()
        != Some(current)
        && let Value::Object(fields) = message
        && let Some(Value::Object(origin)) = fields.get_mut(ENVELOPE)
    {
        origin.remove("output_tokens");
    }
}

/// Marks a newly received message and moves nonportable fields into its
/// original backend's envelope. Already marked messages retain their origin.
pub fn stamp(message: &mut Value, origin: &Origin) {
    let Value::Object(fields) = message else {
        return;
    };
    if fields.contains_key(ENVELOPE) {
        return;
    }
    let mut extras = BTreeMap::new();
    for (key, value) in std::mem::take(fields) {
        if CORE.contains(&key.as_str()) {
            fields.insert(key, value);
        } else {
            extras.insert(key, value);
        }
    }
    let mut envelope = origin.value();
    if let Value::Object(fields) = &mut envelope {
        fields.insert("extras".into(), Value::Object(extras));
    }
    fields.insert(ENVELOPE.into(), envelope);
}

/// The durable fields that any backend can read. Context summaries use this
/// view so opaque native data cannot leak into another model.
pub fn portable(message: &Value) -> Value {
    let Value::Object(fields) = message else {
        return message.clone();
    };
    let mut portable = Value::Object(
        fields
            .iter()
            .filter(|(key, _)| CORE.contains(&key.as_str()))
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect(),
    );
    if let Value::Object(fields) = &mut portable
        && let Some(Value::Array(calls)) = fields.get("tool_calls")
    {
        let canonical = calls
            .iter()
            .map(ToolCall::parse)
            .collect::<Result<Vec<_>, _>>();
        match canonical {
            Ok(canonical) => {
                fields.insert(
                    "tool_calls".into(),
                    Value::Array(canonical.iter().map(ToolCall::value).collect()),
                );
            }
            Err(_) => {
                fields.remove("tool_calls");
            }
        }
    }
    portable
}

/// An explicit history read shows the original native fields and their origin.
/// Inline binary data from old archives is omitted from that text view; the
/// stored archive is untouched and attachment references remain readable.
pub fn history(message: &Value) -> Value {
    let mut shown = message.clone();
    let Value::Object(fields) = &mut shown else {
        return shown;
    };
    for (key, value) in fields {
        if !CORE.contains(&key.as_str()) {
            omit_inline_binary(value);
        }
    }
    shown
}

fn omit_inline_binary(value: &mut Value) {
    match value {
        Value::String(text) if text.starts_with("data:") && text.contains(";base64,") => {
            *value = Value::string("[inline binary data omitted]");
        }
        Value::Array(items) => items.iter_mut().for_each(omit_inline_binary),
        Value::Object(fields) => fields.values_mut().for_each(omit_inline_binary),
        _ => {}
    }
}

/// Returns a backend input projection, restoring opaque fields only for the
/// exact backend and model that wrote them. The backend then materializes
/// and removes local attachment metadata.
pub fn project(message: &Value, current: &Origin) -> Value {
    let Value::Object(fields) = message else {
        return message.clone();
    };
    let Value::Object(mut projected) = portable(message) else {
        unreachable!()
    };
    match fields.get(ENVELOPE) {
        Some(Value::Object(envelope))
            if envelope.get("backend").and_then(Value::as_str)
                == Some(current.backend.as_str())
                && envelope.get("model").and_then(Value::as_str)
                    == Some(current.model.as_str())
                && envelope.get("account").and_then(Value::as_str)
                    == current.account.as_deref() =>
        {
            if let Some(Value::Object(extras)) = envelope.get("extras") {
                projected.extend(
                    extras
                        .iter()
                        .filter(|(key, _)| {
                            !CORE.contains(&key.as_str()) && key.as_str() != ENVELOPE
                        })
                        .map(|(key, value)| (key.clone(), value.clone())),
                );
            }
        }
        _ => {}
    }
    Value::Object(projected)
}

#[cfg(test)]
mod tests;
