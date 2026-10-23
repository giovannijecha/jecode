//! Executable tool calls in the stored conversation.

use crate::json::Value;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: String,
}

impl ToolCall {
    /// Jecode's provider-neutral stored form.
    pub fn value(&self) -> Value {
        Value::object([
            ("id", Value::string(&self.id)),
            ("name", Value::string(&self.name)),
            ("arguments", Value::string(&self.arguments)),
        ])
    }

    /// Reads both Jecode calls and the OpenRouter shape in older archives.
    pub fn parse(value: &Value) -> Result<Self, String> {
        let id = field(value, "id")?;
        if id.is_empty() {
            return Err("Tool call id cannot be empty".into());
        }
        let function = if let Some(function) = value.get("function") {
            if value.get("type").and_then(Value::as_str) != Some("function") {
                return Err("Unsupported legacy tool call type".into());
            }
            function
        } else {
            value
        };
        Ok(Self {
            id: id.into(),
            name: field(function, "name")?.into(),
            arguments: field(function, "arguments")?.into(),
        })
    }
}

fn field<'a>(value: &'a Value, name: &str) -> Result<&'a str, String> {
    value
        .get(name)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("Tool call {name} must be a string"))
}
