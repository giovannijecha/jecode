//! MCP lifecycle, catalog pagination and one-shot tool calls.

use super::transport;
use super::types::{Catalog, Connection, Error, ErrorKind, ServerInfo, Tool};
use super::wire;
use crate::cancel::Cancellation;
use crate::json::Value;
use std::collections::{HashMap, HashSet};

mod handshake;

const MAX_PAGES: usize = 100;
const MAX_TOOLS: usize = 10_000;

#[derive(Clone)]
enum Mode {
    Modern,
    Legacy {
        version: String,
        session: Option<String>,
    },
}

pub struct Client {
    connection: Connection,
    mode: Option<Mode>,
    server: ServerInfo,
    next_id: u64,
    schemas: HashMap<String, Value>,
    catalog_loaded: bool,
}

impl Client {
    pub fn new(connection: Connection) -> Self {
        Self {
            connection,
            mode: None,
            server: ServerInfo::default(),
            next_id: 1,
            schemas: HashMap::new(),
            catalog_loaded: false,
        }
    }

    pub fn catalog(&mut self, cancel: &Cancellation) -> Result<Catalog, Error> {
        match self.list_tools(cancel) {
            Err(error) if self.session_expired(&error) => {
                self.renew(cancel)?;
                self.list_tools(cancel)
            }
            result => result,
        }
    }

    fn list_tools(&mut self, cancel: &Cancellation) -> Result<Catalog, Error> {
        self.connect(cancel)?;
        let modern = matches!(self.mode, Some(Mode::Modern));
        let mut tools = Vec::new();
        let mut cursors = HashSet::new();
        let mut cursor: Option<String> = None;
        for _ in 0..MAX_PAGES {
            let mut params = Value::object([]);
            if let Some(ref cursor) = cursor
                && let Value::Object(ref mut entries) = params
            {
                entries.insert("cursor".into(), Value::string(cursor));
            }
            let result = self.rpc("tools/list", params, None, &[], cancel)?;
            let page = result
                .get("tools")
                .and_then(Value::as_array)
                .ok_or_else(|| wire::protocol("MCP tools/list response has no tools array"))?;
            if tools.len().saturating_add(page.len()) > MAX_TOOLS {
                return Err(wire::protocol("MCP tool catalog is too large"));
            }
            // A malformed entry costs only that tool; header annotations
            // matter only where Modern mode sends them.
            tools.extend(
                page.iter()
                    .filter_map(parse_tool)
                    .filter(|tool| !modern || wire::header_fields(&tool.input_schema).is_ok()),
            );
            cursor = match result.get("nextCursor") {
                None | Some(Value::Null) => None,
                Some(Value::String(value)) if value.is_empty() => None,
                Some(Value::String(value)) => Some(value.clone()),
                _ => return Err(wire::protocol("Invalid MCP pagination cursor")),
            };
            if let Some(ref cursor) = cursor {
                if !cursors.insert(cursor.clone()) {
                    return Err(wire::protocol("Repeated MCP pagination cursor"));
                }
            } else {
                self.schemas = tools
                    .iter()
                    .map(|tool| (tool.name.clone(), tool.input_schema.clone()))
                    .collect();
                self.catalog_loaded = true;
                return Ok(Catalog {
                    server: self.server.clone(),
                    tools,
                });
            }
        }
        Err(wire::protocol("MCP tool catalog exceeded page limit"))
    }

    pub fn call(
        &mut self,
        name: &str,
        arguments: &Value,
        cancel: &Cancellation,
    ) -> Result<Value, Error> {
        if name.is_empty() || name.bytes().any(|byte| byte < 32 || byte == 127) {
            return Err(Error::new(
                ErrorKind::Configuration,
                "Invalid MCP tool name",
            ));
        }
        if !matches!(arguments, Value::Object(_)) {
            return Err(Error::new(
                ErrorKind::Configuration,
                "MCP tool arguments must be an object",
            ));
        }
        match self.call_once(name, arguments, cancel) {
            // An expired session's request was not processed, so one retry is safe.
            Err(error) if self.session_expired(&error) => {
                self.renew(cancel)?;
                self.call_once(name, arguments, cancel)
            }
            result => result,
        }
    }

    fn call_once(
        &mut self,
        name: &str,
        arguments: &Value,
        cancel: &Cancellation,
    ) -> Result<Value, Error> {
        if cancel.requested() {
            return Err(Error::new(ErrorKind::Cancelled, "MCP request cancelled"));
        }
        self.connect(cancel)?;
        let modern = matches!(self.mode, Some(Mode::Modern));
        if modern && !self.catalog_loaded {
            self.catalog(cancel)?;
        }
        let extra_headers = if modern {
            let schema = self.schemas.get(name).ok_or_else(|| {
                Error::new(ErrorKind::Unsupported, "MCP tool is not in the catalog")
            })?;
            wire::parameter_headers(schema, arguments)?
        } else {
            Vec::new()
        };
        if cancel.requested() {
            return Err(Error::new(ErrorKind::Cancelled, "MCP request cancelled"));
        }
        let params = Value::object([
            ("name", Value::string(name)),
            ("arguments", arguments.clone()),
        ]);
        self.rpc("tools/call", params, Some(name), &extra_headers, cancel)
            .and_then(|result| {
                if !matches!(result, Value::Object(_)) {
                    return Err(wire::protocol("MCP tool response is not an object"));
                }
                if modern {
                    match result.get("resultType") {
                        None => {}
                        Some(Value::String(kind)) if kind == "complete" => {}
                        Some(Value::String(kind)) if kind == "input_required" => {
                            return Err(Error::new(
                                ErrorKind::Unsupported,
                                "MCP tool requires additional input to complete",
                            ));
                        }
                        Some(Value::String(_)) => {
                            return Err(Error::new(
                                ErrorKind::Unsupported,
                                "MCP tool returned an unsupported result type",
                            ));
                        }
                        _ => return Err(wire::protocol("Invalid MCP tool result type")),
                    }
                }
                Ok(result)
            })
            .map_err(|mut error| {
                // Definite rejections (4xx, JSON-RPC errors, challenges) keep
                // their kind; anything that may have run on the server is unknown.
                if matches!(
                    error.kind,
                    ErrorKind::Transport | ErrorKind::Cancelled | ErrorKind::Protocol
                ) {
                    error.kind = ErrorKind::UnknownOutcome;
                    error.message = "MCP tool call outcome is unknown".into();
                }
                error
            })
    }

    /// HTTP 404 on a Legacy session means the server dropped it unprocessed.
    fn session_expired(&self, error: &Error) -> bool {
        error.status == Some(404)
            && matches!(
                self.mode,
                Some(Mode::Legacy {
                    session: Some(_),
                    ..
                })
            )
    }

    /// Replaces an expired Legacy session with a fresh initialize.
    fn renew(&mut self, cancel: &Cancellation) -> Result<(), Error> {
        self.mode = None;
        self.catalog_loaded = false;
        self.initialize(cancel)
    }

    fn rpc(
        &mut self,
        method: &str,
        params: Value,
        name: Option<&str>,
        extra_headers: &[(String, String)],
        cancel: &Cancellation,
    ) -> Result<Value, Error> {
        let mode = self
            .mode
            .clone()
            .ok_or_else(|| wire::protocol("MCP connection is not initialized"))?;
        let (id, response) = self.post(method, params, name, Some(&mode), extra_headers, cancel)?;
        reply(&response, id)
    }

    fn post(
        &mut self,
        method: &str,
        params: Value,
        name: Option<&str>,
        mode: Option<&Mode>,
        extra_headers: &[(String, String)],
        cancel: &Cancellation,
    ) -> Result<(u64, transport::Response), Error> {
        let id = self.next_id;
        self.next_id = self
            .next_id
            .checked_add(1)
            .ok_or_else(|| wire::protocol("MCP request ID exhausted"))?;
        let modern = matches!(mode, Some(Mode::Modern));
        let body = wire::request(id, method, params, modern).encode();
        let mut headers = self.headers(method, name, mode)?;
        headers.extend_from_slice(extra_headers);
        let response = transport::request_rpc(
            &self.connection.endpoint,
            "POST",
            &headers,
            Some(body.as_bytes()),
            self.connection.timeout_secs,
            cancel,
            id,
        )?;
        Ok((id, response))
    }

    fn headers(
        &self,
        method: &str,
        name: Option<&str>,
        mode: Option<&Mode>,
    ) -> Result<Vec<(String, String)>, Error> {
        let mut headers = vec![
            (
                "Accept".into(),
                "application/json, text/event-stream".into(),
            ),
            ("Content-Type".into(), "application/json".into()),
        ];
        for (key, value) in &self.connection.headers {
            if key.eq_ignore_ascii_case("Authorization")
                || key.eq_ignore_ascii_case("MCP-Protocol-Version")
                || key.eq_ignore_ascii_case("Mcp-Session-Id")
                || key.eq_ignore_ascii_case("Mcp-Method")
                || key.eq_ignore_ascii_case("Mcp-Name")
                || key.to_ascii_lowercase().starts_with("mcp-param-")
                || key.eq_ignore_ascii_case("Content-Type")
                || key.eq_ignore_ascii_case("Accept")
            {
                return Err(Error::new(
                    ErrorKind::Configuration,
                    "Reserved MCP header configured",
                ));
            }
            headers.push((key.clone(), value.clone()));
        }
        if let Some(bearer) = self.connection.bearer() {
            if bearer.bytes().any(|byte| byte < 33 || byte == 127) {
                return Err(Error::new(
                    ErrorKind::Configuration,
                    "Invalid MCP bearer credential",
                ));
            }
            headers.push(("Authorization".into(), format!("Bearer {bearer}")));
        }
        match mode {
            Some(Mode::Modern) => {
                headers.push(("MCP-Protocol-Version".into(), wire::MODERN.into()));
                headers.push(("Mcp-Method".into(), method.into()));
                if let Some(name) = name {
                    headers.push(("Mcp-Name".into(), wire::header_value(name)));
                }
            }
            Some(Mode::Legacy { version, session }) => {
                headers.push(("MCP-Protocol-Version".into(), version.clone()));
                if let Some(session) = session {
                    headers.push(("Mcp-Session-Id".into(), session.clone()));
                }
            }
            None => {}
        }
        Ok(headers)
    }
}

fn parse_tool(value: &Value) -> Option<Tool> {
    let name = value
        .get("name")
        .and_then(Value::as_str)
        .filter(|name| !name.is_empty())?;
    let schema = value
        .get("inputSchema")
        .filter(|schema| matches!(schema, Value::Object(_)))?;
    Some(Tool {
        name: name.into(),
        title: value
            .get("title")
            .and_then(Value::as_str)
            .map(str::to_owned),
        description: value
            .get("description")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        input_schema: schema.clone(),
        annotations: value.get("annotations").cloned(),
    })
}

fn server_info(value: Option<&Value>) -> ServerInfo {
    ServerInfo {
        name: value
            .and_then(|value| value.get("name"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .into(),
        version: value
            .and_then(|value| value.get("version"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .into(),
        instructions: None,
    }
}

fn content_type(response: &transport::Response) -> Option<&str> {
    response
        .headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("Content-Type"))
        .map(|(_, value)| value.as_str())
}

fn reply(response: &transport::Response, id: u64) -> Result<Value, Error> {
    let decoded = wire::decode(&response.body, content_type(response), id);
    if (200..300).contains(&response.status) {
        return match decoded? {
            wire::Reply::Result(result) => Ok(result),
            wire::Reply::Error(error) => Err(error.into_error()),
        };
    }
    Err(rejection(
        response,
        match decoded {
            Ok(wire::Reply::Error(error)) => Some(error),
            _ => None,
        },
    ))
}

/// Maps a non-2xx response. The transport already turned sign-in challenges
/// into errors; a JSON-RPC error keeps the server's reason, other 4xx
/// responses are definite rejections and the remaining statuses leave the
/// server's state unknown.
fn rejection(response: &transport::Response, error: Option<wire::RpcError>) -> Error {
    let status = response.status;
    let mut error = match error {
        Some(error) => error.into_error(),
        None if (400..500).contains(&status) => Error::new(
            ErrorKind::Remote,
            format!("MCP server rejected the request with HTTP {status}"),
        ),
        None => Error::new(ErrorKind::Transport, format!("MCP HTTP {status}")),
    };
    error.status = Some(status);
    error
}

#[cfg(test)]
mod tests;
