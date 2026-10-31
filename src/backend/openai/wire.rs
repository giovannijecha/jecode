//! Conversion between Jecode's durable transcript and stateless Responses input.

use crate::attachments::{self, Kind, Pool, base64};
use crate::conversation::ToolCall;
use crate::effort::Effort;
use crate::json::Value;
#[cfg(test)]
pub(super) fn request(
    model: &str,
    effort: Effort,
    messages: &[Value],
    with_tools: bool,
    pool: Option<&Pool>,
) -> Result<Value, String> {
    request_with_catalog(
        model,
        effort,
        messages,
        with_tools.then(crate::tools::definitions),
        pool,
    )
}

pub(super) fn request_with_catalog(
    model: &str,
    effort: Effort,
    messages: &[Value],
    catalog: Option<&Value>,
    pool: Option<&Pool>,
) -> Result<Value, String> {
    let mut instructions = Vec::new();
    let mut input = Vec::new();
    let active_start = attachments::active_view_start(messages);
    let mut views = Vec::new();
    for (index, message) in messages.iter().enumerate() {
        let role = message.get("role").and_then(Value::as_str);
        if role != Some("tool") && !views.is_empty() {
            input.push(shown(std::mem::take(&mut views), pool)?);
        }
        match role {
            Some("system" | "developer") => {
                if let Some(content) = message.get("content").and_then(Value::as_str) {
                    instructions.push(content);
                }
            }
            Some("user") => {
                let mut mapped = user(message, pool)?;
                if message.get("jecode_request_state") == Some(&Value::Bool(true))
                    && let Value::Object(fields) = &mut mapped
                {
                    fields.insert("role".into(), Value::string("developer"));
                }
                input.push(mapped);
            }
            Some("assistant") => assistant(message, &mut input)?,
            Some("tool") => {
                let id = field(message, "tool_call_id")?;
                let content = field(message, "content")?;
                input.push(Value::object([
                    ("type", Value::string("function_call_output")),
                    ("call_id", Value::string(id)),
                    ("output", Value::string(content)),
                ]));
                if index >= active_start
                    && let Some(attachment) = attachments::requested(message)
                {
                    views.push(attachment);
                }
            }
            _ => return Err("Conversation contains an unsupported role".into()),
        }
    }
    if !views.is_empty() {
        input.push(shown(views, pool)?);
    }
    let mut request = Value::object([
        ("model", Value::string(model)),
        ("input", Value::Array(input)),
        ("store", Value::Bool(false)),
        ("stream", Value::Bool(true)),
        (
            "include",
            Value::Array(vec![Value::string("reasoning.encrypted_content")]),
        ),
    ]);
    let Value::Object(fields) = &mut request else {
        unreachable!()
    };
    if !instructions.is_empty() {
        fields.insert(
            "instructions".into(),
            Value::string(instructions.join("\n\n")),
        );
    }
    if effort != Effort::Default {
        fields.insert(
            "reasoning".into(),
            Value::object([("effort", Value::string(effort.name()))]),
        );
    }
    if let Some(catalog) = catalog {
        let definitions = catalog.as_array().ok_or("Invalid local tool catalog")?;
        let mut wrapped = Vec::with_capacity(definitions.len());
        for definition in definitions {
            let mut tool = definition.clone();
            let Value::Object(fields) = &mut tool else {
                return Err("Invalid local tool definition".into());
            };
            fields.insert("type".into(), Value::string("function"));
            fields.insert("strict".into(), Value::Bool(false));
            wrapped.push(tool);
        }
        fields.insert(
            "tools".into(),
            Value::Array(vec![Value::object([
                ("type", Value::string("namespace")),
                ("name", Value::string("jecode")),
                (
                    "description",
                    Value::string("Local project files, commands, and Jecode session tools."),
                ),
                ("tools", Value::Array(wrapped)),
            ])]),
        );
    }
    Ok(request)
}

fn shown(attachments: Vec<attachments::Attachment>, pool: Option<&Pool>) -> Result<Value, String> {
    let message = Value::object([
        ("role", Value::string("user")),
        (
            "content",
            Value::string("Attachments requested by the read tool:"),
        ),
        (
            "attachments",
            Value::Array(
                attachments
                    .iter()
                    .map(attachments::Attachment::value)
                    .collect(),
            ),
        ),
    ]);
    user(&message, pool)
}

fn assistant(message: &Value, input: &mut Vec<Value>) -> Result<(), String> {
    if let Some(items) = message.get("openai_items").and_then(Value::as_array) {
        // Native output is replayed only after the origin projection matches.
        // It includes reasoning, assistant phases, and function calls in order.
        input.extend(items.iter().cloned());
        return Ok(());
    }
    let content = message.get("content").and_then(Value::as_str).unwrap_or("");
    if !content.is_empty() {
        input.push(Value::object([
            ("role", Value::string("assistant")),
            ("content", Value::string(content)),
        ]));
    }
    if let Some(calls) = message.get("tool_calls").and_then(Value::as_array) {
        for call in calls {
            let call = ToolCall::parse(call)?;
            input.push(Value::object([
                ("type", Value::string("function_call")),
                ("namespace", Value::string("jecode")),
                ("call_id", Value::string(call.id)),
                ("name", Value::string(call.name)),
                ("arguments", Value::string(call.arguments)),
            ]));
        }
    }
    Ok(())
}

fn user(message: &Value, pool: Option<&Pool>) -> Result<Value, String> {
    let text = attachments::strip(field(message, "content")?);
    let attachments = attachments::of_message(message);
    if attachments.is_empty() {
        return Ok(Value::object([
            ("role", Value::string("user")),
            ("content", Value::string(text)),
        ]));
    }
    let mut manifest = Vec::new();
    let mut parts = Vec::new();
    for (index, attachment) in attachments.iter().enumerate() {
        let reference = attachment.reference();
        let status = match (attachment.kind(), pool) {
            (Kind::Image, Some(pool)) => match pool.view(attachment) {
                Ok(Some((media, bytes))) => {
                    parts.push(Value::object([
                        ("type", Value::string("input_image")),
                        (
                            "image_url",
                            Value::string(format!(
                                "data:{media};base64,{}",
                                base64::encode(&bytes)
                            )),
                        ),
                    ]));
                    "image included".to_owned()
                }
                Ok(None) => "image unavailable in a supported form".into(),
                Err(error) => format!("image unavailable: {error}"),
            },
            (Kind::Pdf, Some(pool)) if attachment.size <= attachments::PDF_INLINE_BYTES => {
                match pool.load(&attachment.id).and_then(|stored| {
                    std::fs::read(stored.path).map_err(|error| error.to_string())
                }) {
                    Ok(bytes) => {
                        parts.push(Value::object([
                            ("type", Value::string("input_file")),
                            ("filename", Value::string(&attachment.name)),
                            (
                                "file_data",
                                Value::string(format!(
                                    "data:application/pdf;base64,{}",
                                    base64::encode(&bytes)
                                )),
                            ),
                        ]));
                        "PDF included".into()
                    }
                    Err(error) => format!("PDF unavailable: {error}"),
                }
            }
            (Kind::Pdf, _) => "PDF not inlined; read the local reference".into(),
            _ => "read the local reference with the read tool".into(),
        };
        manifest.push(format!(
            "{} {} ({}); reference {}; {status}",
            attachment.label(index + 1),
            attachment.name,
            attachment.details(),
            reference
        ));
    }
    let content = if text.trim().is_empty() {
        format!("Attachments:\n{}", manifest.join("\n"))
    } else {
        format!("{text}\n\nAttachments:\n{}", manifest.join("\n"))
    };
    parts.insert(
        0,
        Value::object([
            ("type", Value::string("input_text")),
            ("text", Value::string(content)),
        ]),
    );
    Ok(Value::object([
        ("role", Value::string("user")),
        ("content", Value::Array(parts)),
    ]))
}

fn field<'a>(value: &'a Value, key: &str) -> Result<&'a str, String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("Conversation {key} must be a string"))
}

#[cfg(test)]
mod tests;
