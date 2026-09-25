use super::{Error, identifier};
use crate::json::{self, Value};
use std::collections::BTreeSet;

pub const ENDPOINT: &str = "https://chatgpt.com/backend-api/codex/responses";

pub enum Input {
    User(String),
    /// Completed provider items, including opaque reasoning, in canonical order.
    Assistant(Vec<Value>),
    ToolResult {
        call_id: String,
        output: String,
    },
    ToolImage {
        call_id: String,
        description: String,
        image_url: String,
    },
}

pub struct Tool {
    pub name: String,
    pub description: String,
    pub parameters: Value,
    /// Emit strict only for tool schemas that deliberately opt in.
    pub strict: Option<bool>,
}

pub struct Request {
    pub model: String,
    pub instructions: String,
    pub input: Vec<Input>,
    pub tools: Vec<Tool>,
    /// `None` omits reasoning.effort; it is distinct from the literal `none`.
    pub effort: Option<String>,
}

impl Request {
    /// Serialize the experimental account contract. No credential enters this body.
    pub fn encode(&self, max_bytes: usize) -> Result<String, Error> {
        if !identifier(&self.model)
            || self
                .effort
                .as_deref()
                .is_some_and(|effort| !identifier(effort))
            || self.input.len() > 4096
            || self.tools.len() > 128
        {
            return Err(Error::InvalidRequest);
        }
        let mut input = Vec::new();
        for item in &self.input {
            match item {
                Input::User(text) => input.push(json::object([
                    ("role", string("user")),
                    (
                        "content",
                        Value::Array(vec![json::object([
                            ("type", string("input_text")),
                            ("text", string(text)),
                        ])]),
                    ),
                ])),
                Input::Assistant(items) => {
                    for item in items {
                        if !matches!(item, Value::Object(_)) {
                            return Err(Error::InvalidRequest);
                        }
                        input.push(item.clone());
                    }
                }
                Input::ToolResult { call_id, output } => {
                    if !identifier(call_id) {
                        return Err(Error::InvalidRequest);
                    }
                    input.push(json::object([
                        ("type", string("function_call_output")),
                        ("call_id", string(call_id)),
                        ("output", string(output)),
                    ]));
                }
                Input::ToolImage {
                    call_id,
                    description,
                    image_url,
                } => {
                    if !identifier(call_id) || !image_url.starts_with("data:image/png;base64,") {
                        return Err(Error::InvalidRequest);
                    }
                    input.push(json::object([
                        ("type", string("function_call_output")),
                        ("call_id", string(call_id)),
                        (
                            "output",
                            Value::Array(vec![
                                json::object([
                                    ("type", string("input_text")),
                                    ("text", string(description)),
                                ]),
                                json::object([
                                    ("type", string("input_image")),
                                    ("image_url", string(image_url)),
                                    ("detail", string("high")),
                                ]),
                            ]),
                        ),
                    ]));
                }
            }
            if input.len() > 4096 {
                return Err(Error::InvalidRequest);
            }
        }
        let mut names = BTreeSet::new();
        let mut tools = Vec::new();
        for tool in &self.tools {
            if !identifier(&tool.name)
                || !names.insert(&tool.name)
                || !matches!(tool.parameters, Value::Object(_))
            {
                return Err(Error::InvalidRequest);
            }
            let mut definition = vec![
                ("type", string("function")),
                ("name", string(&tool.name)),
                ("description", string(&tool.description)),
                ("parameters", tool.parameters.clone()),
            ];
            if let Some(strict) = tool.strict {
                definition.push(("strict", Value::Bool(strict)));
            }
            tools.push(json::object(definition));
        }
        let mut reasoning = vec![("summary", string("auto"))];
        if let Some(effort) = &self.effort {
            reasoning.push(("effort", string(effort)));
        }
        let body = json::object([
            ("model", string(&self.model)),
            ("instructions", string(&self.instructions)),
            ("store", Value::Bool(false)),
            ("stream", Value::Bool(true)),
            ("input", Value::Array(input)),
            ("tools", Value::Array(tools)),
            ("tool_choice", string("auto")),
            ("parallel_tool_calls", Value::Bool(true)),
            ("reasoning", json::object(reasoning)),
            (
                "include",
                Value::Array(vec![string("reasoning.encrypted_content")]),
            ),
        ]);
        Ok(json::encode(&body, max_bytes)?)
    }
}

fn string(text: &str) -> Value {
    Value::String(text.to_owned())
}
