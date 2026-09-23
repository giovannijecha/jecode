use super::{Error, field, identifier};
use crate::json::{self, Value};
use std::collections::BTreeSet;

#[derive(Debug, PartialEq, Eq)]
pub enum Status {
    Completed,
    Incomplete,
    Refused,
}

#[derive(Debug, PartialEq, Eq)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: Value,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Usage {
    pub input: Option<u64>,
    pub output: Option<u64>,
    pub cached: Option<u64>,
    pub reasoning: Option<u64>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Response {
    pub id: String,
    pub status: Status,
    pub output: Vec<Value>,
    pub text: String,
    pub tool_calls: Vec<ToolCall>,
    pub usage: Usage,
}
impl Response {
    /// Older v1 steps stored the unseparated projection. Keep those bytes
    /// readable without changing the saved step or its canonical output items.
    pub(crate) fn legacy_text(&self) -> Result<String, Error> {
        Ok(text_parts(&self.output)?
            .iter()
            .map(|part| part.text)
            .collect())
    }
}

pub(super) struct TextPart<'a> {
    pub output_index: usize,
    pub content_index: usize,
    pub item_id: Option<&'a str>,
    pub text: &'a str,
}

pub(super) fn text_parts(output: &[Value]) -> Result<Vec<TextPart<'_>>, Error> {
    let mut parts = Vec::new();
    for (output_index, item) in output.iter().enumerate() {
        if field(item, "type")? != "message" {
            continue;
        }
        let content = item
            .get("content")
            .and_then(Value::array)
            .ok_or(Error::InvalidEvent)?;
        let item_id = item.get("id").and_then(Value::text);
        for (content_index, part) in content.iter().enumerate() {
            let text = match field(part, "type")? {
                "output_text" => field(part, "text")?,
                "refusal" => field(part, "refusal")?,
                _ => continue,
            };
            parts.push(TextPart {
                output_index,
                content_index,
                item_id,
                text,
            });
        }
    }
    Ok(parts)
}

pub(super) fn assemble(
    data: &Value,
    output: Vec<Value>,
    completed: bool,
) -> Result<Response, Error> {
    let id = field(data, "id")?;
    if !identifier(id) {
        return Err(Error::InvalidEvent);
    }
    let expected = if completed { "completed" } else { "incomplete" };
    if field(data, "status")? != expected {
        return Err(Error::InvalidEvent);
    }
    let mut result = Response {
        id: id.to_owned(),
        status: if completed {
            Status::Completed
        } else {
            Status::Incomplete
        },
        output,
        text: String::new(),
        tool_calls: Vec::new(),
        usage: usage(data.get("usage"))?,
    };
    let mut ids = BTreeSet::new();
    let mut item_ids = BTreeSet::new();
    for item in &result.output {
        if let Some(id) = item.get("id") {
            let id = id
                .text()
                .filter(|id| identifier(id))
                .ok_or(Error::InvalidEvent)?;
            if !item_ids.insert(id) {
                return Err(Error::ConflictingOutput);
            }
        }
        match field(item, "type")? {
            "message" => {
                if completed
                    && item
                        .get("status")
                        .is_some_and(|s| s.text() != Some("completed"))
                {
                    return Err(Error::ConflictingOutput);
                }
                if item.get("content").and_then(Value::array).is_none() {
                    return Err(Error::InvalidEvent);
                }
                if completed
                    && item
                        .get("content")
                        .and_then(Value::array)
                        .is_some_and(|parts| {
                            parts.iter().any(|part| {
                                part.get("type").and_then(Value::text) == Some("refusal")
                            })
                        })
                {
                    result.status = Status::Refused;
                }
            }
            "function_call" if completed => {
                if item
                    .get("status")
                    .is_some_and(|s| s.text() != Some("completed"))
                {
                    return Err(Error::InvalidTool);
                }
                let id = field(item, "call_id").map_err(|_| Error::InvalidTool)?;
                let name = field(item, "name").map_err(|_| Error::InvalidTool)?;
                if !identifier(id) || !identifier(name) || !ids.insert(id) {
                    return Err(Error::InvalidTool);
                }
                let arguments = json::parse(field(item, "arguments")?, json::Limits::default())?;
                if !matches!(arguments, Value::Object(_)) {
                    return Err(Error::InvalidTool);
                }
                result.tool_calls.push(ToolCall {
                    id: id.to_owned(),
                    name: name.to_owned(),
                    arguments,
                });
            }
            _ => {} // Retain opaque reasoning and future items without executing them.
        }
    }
    let mut previous = None;
    for part in text_parts(&result.output)? {
        if !result.text.is_empty() && !part.text.is_empty() {
            result
                .text
                .push_str(if previous == Some(part.output_index) {
                    "\n"
                } else {
                    "\n\n"
                });
        }
        result.text.push_str(part.text);
        if !part.text.is_empty() {
            previous = Some(part.output_index);
        }
    }
    if result.status != Status::Completed {
        result.tool_calls.clear();
    }
    Ok(result)
}

fn count(value: Option<&Value>, name: &str) -> Result<Option<u64>, Error> {
    value
        .and_then(|v| v.get(name))
        .map(|v| v.unsigned().ok_or(Error::InvalidUsage))
        .transpose()
}

fn usage(value: Option<&Value>) -> Result<Usage, Error> {
    let value = value.filter(|v| **v != Value::Null);
    if value.is_some_and(|v| !matches!(v, Value::Object(_))) {
        return Err(Error::InvalidUsage);
    }
    for key in ["input_tokens_details", "output_tokens_details"] {
        if value
            .and_then(|v| v.get(key))
            .is_some_and(|v| !matches!(v, Value::Object(_) | Value::Null))
        {
            return Err(Error::InvalidUsage);
        }
    }
    let usage = Usage {
        input: count(value, "input_tokens")?,
        output: count(value, "output_tokens")?,
        cached: count(
            value.and_then(|v| v.get("input_tokens_details")),
            "cached_tokens",
        )?,
        reasoning: count(
            value.and_then(|v| v.get("output_tokens_details")),
            "reasoning_tokens",
        )?,
    };
    if usage.cached.zip(usage.input).is_some_and(|(a, b)| a > b)
        || usage
            .reasoning
            .zip(usage.output)
            .is_some_and(|(a, b)| a > b)
    {
        return Err(Error::InvalidUsage);
    }
    Ok(usage)
}
