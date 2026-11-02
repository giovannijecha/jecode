//! JSON-RPC envelopes and Streamable HTTP response decoding.

use crate::json::{self, Value};
use crate::mcp::types::{Error, ErrorKind};
use std::collections::HashSet;

mod sse;
pub use sse::SseScanner;

pub const MODERN: &str = "2026-07-28";
pub const LEGACY: &str = "2025-11-25";
pub const OLDER: &str = "2025-06-18";
pub const OLDEST: &str = "2025-03-26";

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

/// A decoded JSON-RPC reply to one request.
pub enum Reply {
    Result(Value),
    Error(RpcError),
}

pub struct RpcError {
    pub code: Option<i64>,
    message: String,
    data: Option<Value>,
}

impl RpcError {
    /// Whether a version error offers a revision the `initialize` handshake speaks.
    pub fn offers_legacy(&self) -> bool {
        self.data
            .as_ref()
            .and_then(|data| data.get("supported"))
            .and_then(Value::as_array)
            .is_some_and(|versions| {
                versions
                    .iter()
                    .any(|version| version.as_str().is_some_and(legacy_version))
            })
    }

    pub fn into_error(self) -> Error {
        let Some(code) = self.code else {
            return protocol("Invalid MCP JSON-RPC error");
        };
        let kind = match code {
            -32601 | -32022 => ErrorKind::Unsupported,
            _ => ErrorKind::Remote,
        };
        let mut message = match code {
            -32601 => "MCP method is unsupported".to_owned(),
            -32022 => "MCP protocol version is unsupported".to_owned(),
            code => format!("MCP server returned JSON-RPC error {code}"),
        };
        let detail = bounded(&self.message);
        if !detail.is_empty() {
            message.push_str(": ");
            message.push_str(&detail);
        }
        if let Some(data) = self.data.map(|data| data.encode())
            && data.len() <= DETAIL_LIMIT
        {
            message.push_str(" (data: ");
            message.push_str(&bounded(&data));
            message.push(')');
        }
        Error::new(kind, message)
    }
}

const DETAIL_LIMIT: usize = 500;

/// Server text kept for the model: one line of at most `DETAIL_LIMIT` characters.
fn bounded(text: &str) -> String {
    let text = text.trim();
    let mut out: String = text
        .chars()
        .take(DETAIL_LIMIT)
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    if text.chars().nth(DETAIL_LIMIT).is_some() {
        out.push('…');
    }
    out
}

/// Revisions negotiated through `initialize`, whose HTTP framing Legacy mode uses.
pub fn legacy_version(version: &str) -> bool {
    matches!(version, LEGACY | OLDER | OLDEST)
}

pub fn decode(body: &[u8], content_type: Option<&str>, id: u64) -> Result<Reply, Error> {
    let text = std::str::from_utf8(body).map_err(|_| protocol("MCP response is not UTF-8"))?;
    let media = content_type
        .unwrap_or("application/json")
        .split(';')
        .next()
        .unwrap_or_default()
        .trim();
    if media.eq_ignore_ascii_case("text/event-stream") {
        let mut events = sse::Splitter::default();
        let mut found = None;
        let mut accept = |event: &[u8]| match sse::response(event, id) {
            Some(value) => match found.replace(value) {
                Some(_) => Err(protocol("Duplicate MCP response ID")),
                None => Ok(()),
            },
            None => Ok(()),
        };
        while let Some(event) = events.next(body) {
            accept(event)?;
        }
        accept(events.rest(body))?;
        let value =
            found.ok_or_else(|| protocol("MCP SSE stream ended without the requested response"))?;
        envelope(value, id)
    } else if media.eq_ignore_ascii_case("application/json") || media.is_empty() {
        let value = json::parse(text).map_err(|_| protocol("Invalid MCP response JSON"))?;
        envelope(value, id)
    } else {
        Err(protocol("Unsupported MCP response content type"))
    }
}

fn envelope(value: Value, id: u64) -> Result<Reply, Error> {
    if value.get("jsonrpc").and_then(Value::as_str) != Some("2.0")
        || value.get("id") != Some(&Value::number(id))
    {
        return Err(protocol("Invalid MCP response envelope or ID"));
    }
    match (value.get("result"), value.get("error")) {
        (Some(result), None) => Ok(Reply::Result(result.clone())),
        (None, Some(error)) => Ok(Reply::Error(RpcError {
            code: error.get("code").and_then(|value| match value {
                Value::Number(number) => number.parse::<i64>().ok(),
                _ => None,
            }),
            message: error
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            data: error.get("data").cloned(),
        })),
        _ => Err(protocol("Invalid MCP response result")),
    }
}

pub fn protocol(message: &'static str) -> Error {
    Error::new(ErrorKind::Protocol, message)
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

    fn response(body: &[u8], content_type: Option<&str>, id: u64) -> Result<Value, Error> {
        match decode(body, content_type, id)? {
            Reply::Result(result) => Ok(result),
            Reply::Error(error) => Err(error.into_error()),
        }
    }

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
    fn sse_skips_priming_event_server_request_and_noise() {
        let body = b"id: 1\ndata: \n\ndata: {\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"ping\"}\n\ndata: <html>\n\ndata: {\"jsonrpc\":\"2.0\",\"id\":2,\"result\":{\"x\":1}}";
        assert_eq!(
            response(body, Some("text/event-stream"), 2)
                .unwrap()
                .get("x"),
            Some(&Value::number(1))
        );
        assert_eq!(
            response(b"id: 1\ndata: \n\n", Some("text/event-stream"), 2)
                .unwrap_err()
                .kind,
            ErrorKind::Protocol
        );
    }

    #[test]
    fn server_error_message_is_kept_bounded() {
        let long = "x".repeat(900);
        let body = format!(
            r#"{{"jsonrpc":"2.0","id":1,"error":{{"code":-32602,"message":"bad\nfield {long}","data":{{"field":"title"}}}}}}"#
        );
        let error = response(body.as_bytes(), None, 1).unwrap_err();
        assert_eq!(error.kind, ErrorKind::Remote);
        assert!(
            error
                .message
                .starts_with("MCP server returned JSON-RPC error -32602: bad field x")
        );
        assert!(error.message.contains("(data: {\"field\":\"title\"})"));
        assert!(error.message.len() < 600);
    }

    #[test]
    fn rejects_mismatched_id_and_maps_unsupported_method() {
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
            br#"{"jsonrpc":"2.0","id":1,"error":{"code":-32601,"message":"no such method"}}"#,
            None,
            1,
        )
        .unwrap_err();
        assert_eq!(error.kind, ErrorKind::Unsupported);
        assert_eq!(error.message, "MCP method is unsupported: no such method");
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
