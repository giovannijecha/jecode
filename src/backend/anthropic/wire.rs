//! Canonical transcript and attachment mapping for Messages.

use crate::attachments::{self, Attachment, Kind, Pool, base64};
use crate::conversation::ToolCall;
use crate::effort::Effort;
use crate::json::Value;

const COMPATIBILITY_SYSTEM: &str = "You are Claude Code, Anthropic's official CLI for Claude.";

pub(super) fn request(
    model: &str,
    effort: Effort,
    messages: &[Value],
    catalog: Option<&Value>,
    output_tokens: Option<usize>,
    thinking_off: Option<&str>,
    pool: Option<&Pool>,
) -> Result<Value, String> {
    let mut instructions = vec![COMPATIBILITY_SYSTEM.to_owned()];
    let mut wire = Vec::new();
    let mut results = Vec::new();
    let active_start = attachments::active_view_start(messages);
    let mut views = Vec::new();
    let mut volatile_start = None;
    for (index, message) in messages.iter().enumerate() {
        let role = message.get("role").and_then(Value::as_str);
        if role != Some("tool") && !results.is_empty() {
            wire.push(Value::object([
                ("role", Value::string("user")),
                ("content", Value::Array(std::mem::take(&mut results))),
            ]));
            if !views.is_empty() {
                wire.push(shown(std::mem::take(&mut views), pool)?);
            }
        }
        match role {
            Some("system" | "developer") => {
                if let Some(content) = message.get("content").and_then(Value::as_str)
                    && !content.is_empty()
                {
                    instructions.push(content.to_owned());
                }
            }
            Some("user") => {
                let request_state = message.get("jecode_request_state") == Some(&Value::Bool(true));
                if request_state {
                    volatile_start = Some(wire.len());
                }
                let mut mapped = user(message, pool)?;
                if request_state
                    && supports_mid_system(model)
                    && let Value::Object(fields) = &mut mapped
                {
                    fields.insert("role".into(), Value::string("system"));
                }
                wire.push(mapped);
            }
            Some("assistant") => wire.push(assistant(message)?),
            Some("tool") => {
                results.push(Value::object([
                    ("type", Value::string("tool_result")),
                    (
                        "tool_use_id",
                        Value::string(field(message, "tool_call_id")?),
                    ),
                    ("content", Value::string(field(message, "content")?)),
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
    if !results.is_empty() {
        wire.push(Value::object([
            ("role", Value::string("user")),
            ("content", Value::Array(results)),
        ]));
    }
    if !views.is_empty() {
        wire.push(shown(views, pool)?);
    }
    // A tool-result batch can exceed the 20-block lookback window. Preserve
    // recent user-message ends, including earlier batched tool results.
    let durable_end = volatile_start.unwrap_or(wire.len());
    let slots = if volatile_start.is_some() { 3 } else { 2 };
    let mut marked = 0;
    for message in wire[..durable_end].iter_mut().rev() {
        if message.get("role").and_then(Value::as_str) == Some("user") && cache_breakpoint(message)
        {
            marked += 1;
            if marked == slots {
                break;
            }
        }
    }
    let system: Vec<Value> = instructions
        .into_iter()
        .map(|text| {
            Value::object([
                ("type", Value::string("text")),
                ("text", Value::string(text)),
            ])
        })
        .collect();
    let mut system = system;
    if let Some(Value::Object(last)) = system.last_mut() {
        last.insert("cache_control".into(), ephemeral());
    }
    let mut request = Value::object([
        ("model", Value::string(model)),
        ("system", Value::Array(system)),
        ("messages", Value::Array(wire)),
        (
            "max_tokens",
            Value::number(output_tokens.unwrap_or(8192).max(1)),
        ),
        ("stream", Value::Bool(true)),
    ]);
    let Value::Object(fields) = &mut request else {
        unreachable!()
    };
    if volatile_start.is_none() {
        fields.insert("cache_control".into(), ephemeral());
    }
    match effort {
        Effort::Default => {}
        Effort::None => {
            let mode = thinking_off.ok_or("Anthropic model has no verified thinking-off mode")?;
            fields.insert(
                "thinking".into(),
                Value::object([("type", Value::string(mode))]),
            );
        }
        Effort::Low | Effort::Medium | Effort::High | Effort::Xhigh | Effort::Max => {
            fields.insert(
                "thinking".into(),
                Value::object([("type", Value::string("adaptive"))]),
            );
            fields.insert(
                "output_config".into(),
                Value::object([("effort", Value::string(effort.name()))]),
            );
        }
        _ => {
            return Err(format!(
                "Anthropic does not support {} effort",
                effort.name()
            ));
        }
    }
    if let Some(catalog) = catalog {
        let tools = catalog.as_array().ok_or("Invalid local tool catalog")?;
        let mut mapped = Vec::with_capacity(tools.len());
        for tool in tools {
            mapped.push(Value::object([
                ("name", Value::string(field(tool, "name")?)),
                ("description", Value::string(field(tool, "description")?)),
                (
                    "input_schema",
                    tool.get("parameters")
                        .ok_or("Tool has no parameters")?
                        .clone(),
                ),
            ]));
        }
        fields.insert("tools".into(), Value::Array(mapped));
    }
    Ok(request)
}

fn ephemeral() -> Value {
    Value::object([("type", Value::string("ephemeral"))])
}

fn supports_mid_system(model: &str) -> bool {
    const MODELS: &[&str] = &[
        "claude-opus-5-5",
        "claude-opus-4-8",
        "claude-opus-5",
        "claude-sonnet-5-5",
        "claude-haiku-5-5",
        "claude-fable-5-1",
        "claude-fable-5",
        "claude-mythos-5-1",
        "claude-mythos-5",
    ];
    MODELS.iter().any(|known| {
        model == *known
            || model.strip_prefix(known).is_some_and(|suffix| {
                suffix.strip_prefix('-').is_some_and(|date| {
                    date.len() == 8 && date.bytes().all(|byte| byte.is_ascii_digit())
                })
            })
    })
}

fn cache_breakpoint(message: &mut Value) -> bool {
    let Value::Object(fields) = message else {
        return false;
    };
    let Some(content) = fields.get_mut("content") else {
        return false;
    };
    match content {
        Value::String(text) if !text.is_empty() => {
            let text = text.clone();
            *content = Value::Array(vec![Value::object([
                ("type", Value::string("text")),
                ("text", Value::string(text)),
                ("cache_control", ephemeral()),
            ])]);
            true
        }
        Value::Array(blocks) => {
            let Some(Value::Object(block)) = blocks.iter_mut().rev().find(|block| {
                match block.get("type").and_then(Value::as_str) {
                    Some("text") => block
                        .get("text")
                        .and_then(Value::as_str)
                        .is_some_and(|text| !text.is_empty()),
                    Some("tool_use" | "tool_result" | "image" | "document") => true,
                    _ => false,
                }
            }) else {
                return false;
            };
            block.insert("cache_control".into(), ephemeral());
            true
        }
        _ => false,
    }
}

fn assistant(message: &Value) -> Result<Value, String> {
    if let Some(blocks) = message.get("anthropic_blocks").and_then(Value::as_array) {
        return Ok(Value::object([
            ("role", Value::string("assistant")),
            ("content", Value::Array(blocks.to_vec())),
        ]));
    }
    let mut blocks = Vec::new();
    if let Some(content) = message.get("content").and_then(Value::as_str)
        && !content.is_empty()
    {
        blocks.push(Value::object([
            ("type", Value::string("text")),
            ("text", Value::string(content)),
        ]));
    }
    if let Some(calls) = message.get("tool_calls").and_then(Value::as_array) {
        for call in calls {
            let call = ToolCall::parse(call)?;
            blocks.push(Value::object([
                ("type", Value::string("tool_use")),
                ("id", Value::string(call.id)),
                ("name", Value::string(call.name)),
                ("input", crate::json::parse(&call.arguments)?),
            ]));
        }
    }
    Ok(Value::object([
        ("role", Value::string("assistant")),
        ("content", Value::Array(blocks)),
    ]))
}

fn shown(attachments: Vec<Attachment>, pool: Option<&Pool>) -> Result<Value, String> {
    user(
        &Value::object([
            ("role", Value::string("user")),
            (
                "content",
                Value::string("Attachments requested by the read tool:"),
            ),
            (
                "attachments",
                Value::Array(attachments.iter().map(Attachment::value).collect()),
            ),
        ]),
        pool,
    )
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
    let mut blocks = Vec::new();
    for (index, attachment) in attachments.iter().enumerate() {
        let status = match (attachment.kind(), pool) {
            (Kind::Image, Some(pool)) => match pool.view(attachment) {
                Ok(Some((media, bytes)))
                    if matches!(
                        media.as_str(),
                        "image/jpeg" | "image/png" | "image/gif" | "image/webp"
                    ) =>
                {
                    blocks.push(binary("image", &media, &bytes));
                    "image included".to_owned()
                }
                Ok(_) => "image unavailable in a supported form".into(),
                Err(error) => format!("image unavailable: {error}"),
            },
            (Kind::Pdf, Some(pool)) if attachment.size <= attachments::PDF_INLINE_BYTES => {
                match pool.load(&attachment.id).and_then(|stored| {
                    std::fs::read(stored.path).map_err(|error| error.to_string())
                }) {
                    Ok(bytes) => {
                        blocks.push(binary("document", "application/pdf", &bytes));
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
            attachment.reference()
        ));
    }
    let text = if text.trim().is_empty() {
        format!("Attachments:\n{}", manifest.join("\n"))
    } else {
        format!("{text}\n\nAttachments:\n{}", manifest.join("\n"))
    };
    blocks.insert(
        0,
        Value::object([
            ("type", Value::string("text")),
            ("text", Value::string(text)),
        ]),
    );
    Ok(Value::object([
        ("role", Value::string("user")),
        ("content", Value::Array(blocks)),
    ]))
}

fn binary(kind: &str, media: &str, bytes: &[u8]) -> Value {
    Value::object([
        ("type", Value::string(kind)),
        (
            "source",
            Value::object([
                ("type", Value::string("base64")),
                ("media_type", Value::string(media)),
                ("data", Value::string(base64::encode(bytes))),
            ]),
        ),
    ])
}

fn field<'a>(value: &'a Value, key: &str) -> Result<&'a str, String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("Conversation {key} must be a string"))
}
