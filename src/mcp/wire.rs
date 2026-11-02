//! JSON-RPC envelopes and Streamable HTTP response decoding.

use crate::json::{self, Value};
use crate::mcp::types::{Error, ErrorKind};
use std::collections::HashSet;

pub const MODERN: &str = "2026-07-28";
pub const LEGACY: &str = "2025-11-25";
pub const OLDER: &str = "2025-06-18";

pub fn request(id: u64, method: &str, mut params: Value, modern: bool) -> Value {
    if modern && let Value::Object(ref mut entries) = params {
        entries.insert(
            "_meta".into(),
            Value::object([
                (
                    "io.modelcontextprotocol/protocolVersion",
                    Value::string(MODERN),
                ),
                (
                    "io.modelcontextprotocol/clientCapabilities",
                    Value::object([]),
                ),
                (
                    "io.modelcontextprotocol/clientInfo",
                    Value::object([
                        ("name", Value::string("jecode")),
                        ("version", Value::string(env!("CARGO_PKG_VERSION"))),
                    ]),
                ),
            ]),
        );
    }
    Value::object([
        ("jsonrpc", Value::string("2.0")),
        ("id", Value::number(id)),
        ("method", Value::string(method)),
        ("params", params),
    ])
}

pub fn notification(method: &str) -> Value {
    Value::object([
        ("jsonrpc", Value::string("2.0")),
        ("method", Value::string(method)),
    ])
}

pub fn response(body: &[u8], content_type: Option<&str>, id: u64) -> Result<Value, Error> {
    let text = std::str::from_utf8(body).map_err(|_| protocol("MCP response is not UTF-8"))?;
    let media = content_type
        .unwrap_or("application/json")
        .split(';')
        .next()
        .unwrap_or_default()
        .trim();
    if media.eq_ignore_ascii_case("text/event-stream") {
        let mut found = None;
        let mut data = String::new();
        for line in text.lines().chain(std::iter::once("")) {
            let line = line.trim_end_matches('\r');
            if line.is_empty() {
                if !data.is_empty() {
                    let value = json::parse(data.trim_end_matches('\n'))
                        .map_err(|_| protocol("Invalid MCP SSE JSON"))?;
                    if value.get("id") == Some(&Value::number(id)) {
                        if found.is_some() {
                            return Err(protocol("Duplicate MCP response ID"));
                        }
                        found = Some(envelope(value, id)?);
                    }
                    data.clear();
                }
            } else if let Some(part) = line.strip_prefix("data:") {
                data.push_str(part.strip_prefix(' ').unwrap_or(part));
                data.push('\n');
            }
        }
        found.ok_or_else(|| protocol("MCP SSE stream ended without the requested response"))
    } else if media.eq_ignore_ascii_case("application/json") || media.is_empty() {
        let value = json::parse(text).map_err(|_| protocol("Invalid MCP response JSON"))?;
        envelope(value, id)
    } else {
        Err(protocol("Unsupported MCP response content type"))
    }
}

fn envelope(value: Value, id: u64) -> Result<Value, Error> {
    if value.get("jsonrpc").and_then(Value::as_str) != Some("2.0")
        || value.get("id") != Some(&Value::number(id))
    {
        return Err(protocol("Invalid MCP response envelope or ID"));
    }
    match (value.get("result"), value.get("error")) {
        (Some(result), None) => Ok(result.clone()),
        (None, Some(error)) => {
            let code = error.get("code").and_then(|value| match value {
                Value::Number(number) => number.parse::<i64>().ok(),
                _ => None,
            });
            let kind = match code {
                Some(-32601 | -32022) => ErrorKind::Unsupported,
                Some(_) => ErrorKind::Remote,
                None => ErrorKind::Protocol,
            };
            Err(Error::new(
                kind,
                match code {
                    Some(-32601) => "MCP method is unsupported".into(),
                    Some(-32022) => "MCP protocol version is unsupported".into(),
                    Some(code) => format!("MCP server returned JSON-RPC error {code}"),
                    None => "Invalid MCP JSON-RPC error".into(),
                },
            ))
        }
        _ => Err(protocol("Invalid MCP response result")),
    }
}

pub fn protocol(message: &'static str) -> Error {
    Error::new(ErrorKind::Protocol, message)
}

pub fn error_code(body: &[u8], id: u64) -> Option<i64> {
    let value = json::parse(std::str::from_utf8(body).ok()?).ok()?;
    if value.get("jsonrpc")?.as_str()? != "2.0" || value.get("id")? != &Value::number(id) {
        return None;
    }
    match value.get("error")?.get("code")? {
        Value::Number(number) => number.parse().ok(),
        _ => None,
    }
}

pub fn offered_legacy(body: &[u8], id: u64) -> bool {
    let Ok(text) = std::str::from_utf8(body) else {
        return false;
    };
    let Ok(value) = json::parse(text) else {
        return false;
    };
    value.get("jsonrpc").and_then(Value::as_str) == Some("2.0")
        && value.get("id") == Some(&Value::number(id))
        && value.get("error").and_then(|error| error.get("code")) == Some(&Value::number(-32022))
        && value
            .get("error")
            .and_then(|error| error.get("data"))
            .and_then(|data| data.get("supported"))
            .and_then(Value::as_array)
            .is_some_and(|versions| {
                versions
                    .iter()
                    .any(|version| matches!(version.as_str(), Some(LEGACY | OLDER)))
            })
}

#[derive(Default)]
pub struct SseScanner {
    scan: usize,
    line_start: usize,
    event_start: usize,
}

impl SseScanner {
    /// Returns the byte boundary of a complete SSE event with the requested ID.
    pub fn response_end(&mut self, body: &[u8], id: u64) -> Option<usize> {
        while let Some(offset) = body
            .get(self.scan..)?
            .iter()
            .position(|byte| *byte == b'\n')
        {
            let end = self.scan + offset;
            let line = body
                .get(self.line_start..end)?
                .strip_suffix(b"\r")
                .unwrap_or(&body[self.line_start..end]);
            self.scan = end + 1;
            if line.is_empty() {
                let event = &body[self.event_start..self.line_start];
                self.event_start = self.scan;
                self.line_start = self.scan;
                let mut data = Vec::new();
                for line in event.split(|byte| *byte == b'\n') {
                    let line = line.strip_suffix(b"\r").unwrap_or(line);
                    if let Some(value) = line.strip_prefix(b"data:") {
                        let value = value.strip_prefix(b" ").unwrap_or(value);
                        if !data.is_empty() {
                            data.push(b'\n');
                        }
                        data.extend_from_slice(value);
                    }
                }
                if let Ok(text) = std::str::from_utf8(&data)
                    && let Ok(value) = json::parse(text)
                    && value.get("id") == Some(&Value::number(id))
                {
                    return Some(self.scan);
                }
            } else {
                self.line_start = self.scan;
            }
        }
        self.scan = body.len();
        None
    }
}

pub fn header_value(value: &str) -> String {
    let ascii = value.bytes().all(|byte| (32..=126).contains(&byte));
    let padded = value.starts_with([' ', '\t']) || value.ends_with([' ', '\t']);
    if ascii && !padded && !value.starts_with("=?base64?") {
        value.to_owned()
    } else {
        format!(
            "=?base64?{}?=",
            crate::attachments::base64::encode(value.as_bytes())
        )
    }
}

pub fn header_fields(schema: &Value) -> Result<Vec<(Vec<String>, String, String)>, Error> {
    fn visit(
        schema: &Value,
        path: &mut Vec<String>,
        fields: &mut Vec<(Vec<String>, String, String)>,
    ) -> Result<(), Error> {
        if path.len() > 16 {
            return Err(protocol("MCP tool header path is too deep"));
        }
        if let Some(value) = schema.get("x-mcp-header") {
            let name = value
                .as_str()
                .filter(|name| {
                    !name.is_empty()
                        && name.bytes().all(|byte| {
                            byte.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&byte)
                        })
                })
                .ok_or_else(|| protocol("Invalid MCP tool header name"))?;
            let kind = schema
                .get("type")
                .and_then(Value::as_str)
                .filter(|kind| matches!(*kind, "string" | "integer" | "boolean"))
                .ok_or_else(|| protocol("Invalid MCP tool header type"))?;
            if path.is_empty() {
                return Err(protocol("Invalid MCP tool header location"));
            }
            fields.push((path.clone(), name.to_owned(), kind.to_owned()));
        }
        if let Some(Value::Object(properties)) = schema.get("properties") {
            for (name, child) in properties {
                path.push(name.clone());
                visit(child, path, fields)?;
                path.pop();
            }
        }
        Ok(())
    }
    fn count(value: &Value) -> usize {
        match value {
            Value::Object(entries) => {
                usize::from(entries.contains_key("x-mcp-header"))
                    + entries.values().map(count).sum::<usize>()
            }
            Value::Array(values) => values.iter().map(count).sum(),
            _ => 0,
        }
    }
    let mut fields = Vec::new();
    visit(schema, &mut Vec::new(), &mut fields)?;
    if fields.len() != count(schema) {
        return Err(protocol("Misplaced MCP tool header annotation"));
    }
    let mut names = HashSet::new();
    if fields
        .iter()
        .any(|(_, name, _)| !names.insert(name.to_ascii_lowercase()))
    {
        return Err(protocol("Duplicate MCP tool header name"));
    }
    Ok(fields)
}

pub fn parameter_headers(
    schema: &Value,
    arguments: &Value,
) -> Result<Vec<(String, String)>, Error> {
    let mut headers = Vec::new();
    for (path, name, kind) in header_fields(schema)? {
        let mut value = Some(arguments);
        for part in &path {
            value = value.and_then(|value| value.get(part));
        }
        let Some(value) = value else { continue };
        let text = match (kind.as_str(), value) {
            (_, Value::Null) => continue,
            ("string", Value::String(text)) => text.clone(),
            ("boolean", Value::Bool(boolean)) => boolean.to_string(),
            ("integer", Value::Number(number))
                if number
                    .parse::<i64>()
                    .is_ok_and(|number| number.unsigned_abs() <= 9_007_199_254_740_991) =>
            {
                number.clone()
            }
            _ => {
                return Err(Error::new(
                    ErrorKind::Configuration,
                    "Invalid MCP tool header argument",
                ));
            }
        };
        headers.push((format!("Mcp-Param-{name}"), header_value(&text)));
    }
    Ok(headers)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_fragmented_sse_event_with_other_messages() {
        let body = b": comment\r\nevent: message\r\ndata: {\"jsonrpc\":\"2.0\",\"id\":2,\"result\":{\"x\":1}}\r\n\r\ndata: {\"jsonrpc\":\"2.0\",\"id\":3,\"result\":{\"x\":2}}\r\n\r\n";
        assert_eq!(
            response(body, Some("text/event-stream; charset=utf-8"), 2)
                .unwrap()
                .get("x"),
            Some(&Value::number(1))
        );
    }

    #[test]
    fn rejects_mismatched_id_and_untrusted_error_message() {
        assert_eq!(
            response(
                br#"{"jsonrpc":"2.0","id":2,"result":{}}"#,
                Some("application/json"),
                1
            )
            .unwrap_err()
            .kind,
            ErrorKind::Protocol
        );
        let error = response(
            br#"{"jsonrpc":"2.0","id":1,"error":{"code":-32601,"message":"secret"}}"#,
            None,
            1,
        )
        .unwrap_err();
        assert_eq!(error.kind, ErrorKind::Unsupported);
        assert!(!error.message.contains("secret"));
    }

    #[test]
    fn tool_parameter_headers_validate_paths_and_encode_values() {
        let schema = Value::object([
            ("type", Value::string("object")),
            (
                "properties",
                Value::object([(
                    "nested",
                    Value::object([
                        ("type", Value::string("object")),
                        (
                            "properties",
                            Value::object([(
                                "label",
                                Value::object([
                                    ("type", Value::string("string")),
                                    ("x-mcp-header", Value::string("Label")),
                                ]),
                            )]),
                        ),
                    ]),
                )]),
            ),
        ]);
        let arguments = Value::object([(
            "nested",
            Value::object([("label", Value::string("ciao 世界"))]),
        )]);
        let headers = parameter_headers(&schema, &arguments).unwrap();
        assert_eq!(
            headers,
            vec![("Mcp-Param-Label".into(), header_value("ciao 世界"))]
        );
        assert!(headers[0].1.starts_with("=?base64?"));
        let invalid = Value::object([(
            "oneOf",
            Value::Array(vec![Value::object([(
                "x-mcp-header",
                Value::string("Bad"),
            )])]),
        )]);
        assert!(header_fields(&invalid).is_err());
    }
}
