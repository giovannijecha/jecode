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

    /// Reads Jecode's provider-neutral stored form.
    pub fn parse(value: &Value) -> Result<Self, String> {
        if value.get("function").is_some() {
            return Err("Unsupported stored tool call shape".into());
        }
        let id = field(value, "id")?;
        if id.is_empty() {
            return Err("Tool call id cannot be empty".into());
        }
        Ok(Self {
            id: id.into(),
            name: field(value, "name")?.into(),
            arguments: field(value, "arguments")?.into(),
        })
    }
}

fn field<'a>(value: &'a Value, name: &str) -> Result<&'a str, String> {
    value
        .get(name)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("Tool call {name} must be a string"))
}
