//! Models sometimes send a nested array or object as JSON text, such as an
//! edit's `files` list. Where a built-in tool's schema expects that type and
//! the text parses to it, the call uses the value, so the model does not pay
//! a request to resend it.

use crate::json::{self, Value};

/// The call's arguments with JSON-text arrays and objects decoded where the
/// tool's schema expects them; `None` when nothing changed or the tool is not
/// a built-in one.
pub(crate) fn arguments(name: &str, arguments: &str) -> Option<String> {
    let schema = super::schema::definitions()
        .as_array()?
        .iter()
        .find(|tool| tool.get("name").and_then(Value::as_str) == Some(name))?
        .get("parameters")?;
    let mut value = json::parse(arguments).ok()?;
    coerce(&mut value, schema).then(|| value.encode())
}

fn coerce(value: &mut Value, schema: &Value) -> bool {
    let mut changed = false;
    if let Value::String(text) = value
        && let Some(expected) = schema.get("type").and_then(Value::as_str)
        && matches!(expected, "array" | "object")
        && let Ok(parsed) = json::parse(text.trim())
        && matches!(
            (&parsed, expected),
            (Value::Array(_), "array") | (Value::Object(_), "object")
        )
    {
        *value = parsed;
        changed = true;
    }
    match value {
        Value::Array(items) => {
            if let Some(items_schema) = schema.get("items") {
                for item in items {
                    changed |= coerce(item, items_schema);
                }
            }
        }
        Value::Object(fields) => {
            if let Some(Value::Object(properties)) = schema.get("properties") {
                for (key, field) in fields.iter_mut() {
                    if let Some(property) = properties.get(key) {
                        changed |= coerce(field, property);
                    }
                }
            }
        }
        _ => {}
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_text_becomes_the_array_the_schema_expects_at_every_level() {
        let files = r#"[{"path":"a.rs","edits":"[{\"old_text\":\"a\",\"new_text\":\"b\"}]"}]"#;
        let call = Value::object([("files", Value::string(files))]).encode();
        let fixed = json::parse(&arguments("edit", &call).unwrap()).unwrap();
        let entry = &fixed.get("files").unwrap().as_array().unwrap()[0];
        assert_eq!(entry.get("path").and_then(Value::as_str), Some("a.rs"));
        let edits = entry.get("edits").unwrap().as_array().unwrap();
        assert_eq!(edits[0].get("new_text").and_then(Value::as_str), Some("b"));
    }

    #[test]
    fn strings_the_schema_expects_and_unknown_tools_stay_unchanged() {
        // A string field stays text even when it looks like JSON.
        let call = Value::object([
            ("path", Value::string("a.rs")),
            ("content", Value::string("[1, 2]")),
        ])
        .encode();
        assert_eq!(arguments("write", &call), None);
        // Text that is not the expected type is left for the tool to report.
        let call = Value::object([("files", Value::string("not json"))]).encode();
        assert_eq!(arguments("edit", &call), None);
        assert_eq!(arguments("mcp_server_tool", "{\"files\":\"[]\"}"), None);
        assert_eq!(arguments("edit", "not an object"), None);
    }
}
