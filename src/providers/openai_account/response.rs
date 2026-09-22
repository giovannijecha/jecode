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
                let parts = item
                    .get("content")
                    .and_then(Value::array)
                    .ok_or(Error::InvalidEvent)?;
                for part in parts {
                    match field(part, "type")? {
                        "output_text" => result.text.push_str(field(part, "text")?),
                        "refusal" => {
                            result.text.push_str(field(part, "refusal")?);
                            if completed {
                                result.status = Status::Refused;
                            }
                        }
                        _ => {}
                    }
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
