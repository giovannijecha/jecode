//! MCP lifecycle, catalog pagination and one-shot tool calls.

use super::transport;
use super::types::{Catalog, Connection, Error, ErrorKind, ServerInfo, Tool};
use super::wire;
use crate::cancel::Cancellation;
use crate::json::Value;
use std::collections::{HashMap, HashSet};

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
        self.connect(cancel)?;
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
            let result = self.rpc("tools/list", params, None, cancel)?;
            let page = result
                .get("tools")
                .and_then(Value::as_array)
                .ok_or_else(|| wire::protocol("MCP tools/list response has no tools array"))?;
            if tools.len().saturating_add(page.len()) > MAX_TOOLS {
                return Err(wire::protocol("MCP tool catalog is too large"));
            }
            for value in page {
                let tool = parse_tool(value)?;
                if wire::header_fields(&tool.input_schema).is_ok() {
                    tools.push(tool);
                }
            }
            cursor = match result.get("nextCursor") {
                None | Some(Value::Null) => None,
                Some(Value::String(value)) if !value.is_empty() => Some(value.clone()),
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
        let mode = self
            .mode
            .clone()
            .ok_or_else(|| wire::protocol("MCP connection is not initialized"))?;
        self.send(
            "tools/call",
            params,
            Some(name),
            Some(&mode),
            &extra_headers,
            cancel,
        )
        .and_then(|(result, _)| {
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

    fn connect(&mut self, cancel: &Cancellation) -> Result<(), Error> {
        if self.mode.is_some() {
            return Ok(());
        }
        transport::validate_endpoint(&self.connection.endpoint)?;
        let probe = self.rpc_with_mode(
            "server/discover",
            Value::object([]),
            None,
            &Mode::Modern,
            cancel,
        );
        match probe {
            Ok(result) => {
                let supported = result
                    .get("supportedVersions")
                    .and_then(Value::as_array)
                    .ok_or_else(|| wire::protocol("Invalid MCP discovery versions"))?;
                if supported
                    .iter()
                    .any(|version| version.as_str() == Some(wire::MODERN))
                {
                    self.server = server_info(
                        result
                            .get("_meta")
                            .and_then(|meta| meta.get("io.modelcontextprotocol/serverInfo")),
                    );
                    self.server.instructions = result
                        .get("instructions")
                        .and_then(Value::as_str)
                        .map(str::to_owned);
                    self.mode = Some(Mode::Modern);
                    return Ok(());
                }
            }
            Err(error)
                if error.message == "MCP legacy discovery probe rejected"
                    || error.message == "MCP legacy protocol version offered"
                    || error.message == "MCP method is unsupported" => {}
            Err(error) => return Err(error),
        }
        self.initialize(cancel)
    }

    fn initialize(&mut self, cancel: &Cancellation) -> Result<(), Error> {
        let params = Value::object([
            ("protocolVersion", Value::string(wire::LEGACY)),
            ("capabilities", Value::object([])),
            (
                "clientInfo",
                Value::object([
                    ("name", Value::string("jecode")),
                    ("version", Value::string(env!("CARGO_PKG_VERSION"))),
                ]),
            ),
        ]);
        let (result, response) = self.send("initialize", params, None, None, &[], cancel)?;
        let version = result
            .get("protocolVersion")
            .and_then(Value::as_str)
            .ok_or_else(|| wire::protocol("MCP initialize response has no protocol version"))?;
        if !matches!(version, wire::LEGACY | wire::OLDER) {
            return Err(Error::new(
                ErrorKind::Unsupported,
                "MCP server protocol version is unsupported",
            ));
        }
        let session = response
            .headers
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case("Mcp-Session-Id"))
            .map(|(_, value)| value.clone());
        if session.as_ref().is_some_and(|value| {
            value.is_empty() || value.bytes().any(|byte| byte < 33 || byte == 127)
        }) {
            return Err(wire::protocol("Invalid MCP session ID"));
        }
        let mode = Mode::Legacy {
            version: version.into(),
            session,
        };
        self.server = server_info(result.get("serverInfo"));
        self.server.instructions = result
            .get("instructions")
            .and_then(Value::as_str)
            .map(str::to_owned);
        let body = wire::notification("notifications/initialized").encode();
        let headers = self.headers("notifications/initialized", None, Some(&mode))?;
        let response = transport::request(
            &self.connection.endpoint,
            "POST",
            &headers,
            Some(body.as_bytes()),
            self.connection.timeout_secs,
            cancel,
        )?;
        if !(200..300).contains(&response.status) {
            return Err(http_error(response.status));
        }
        self.mode = Some(mode);
        Ok(())
    }

    fn rpc(
        &mut self,
        method: &str,
        params: Value,
        name: Option<&str>,
        cancel: &Cancellation,
    ) -> Result<Value, Error> {
        let mode = self
            .mode
            .clone()
            .ok_or_else(|| wire::protocol("MCP connection is not initialized"))?;
        self.rpc_with_mode(method, params, name, &mode, cancel)
    }

    fn rpc_with_mode(
        &mut self,
        method: &str,
        params: Value,
        name: Option<&str>,
        mode: &Mode,
        cancel: &Cancellation,
    ) -> Result<Value, Error> {
        self.send(method, params, name, Some(mode), &[], cancel)
            .map(|(result, _)| result)
    }

    fn send(
        &mut self,
        method: &str,
        params: Value,
        name: Option<&str>,
        mode: Option<&Mode>,
        extra_headers: &[(String, String)],
        cancel: &Cancellation,
    ) -> Result<(Value, transport::Response), Error> {
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
        if !(200..300).contains(&response.status) {
            if response.status == 400 && method == "server/discover" {
                return match wire::error_code(&response.body, id) {
                    Some(-32022) if wire::offered_legacy(&response.body, id) => Err(Error::new(
                        ErrorKind::Unsupported,
                        "MCP legacy protocol version offered",
                    )),
                    Some(-32022) => Err(Error::new(
                        ErrorKind::Unsupported,
                        "MCP modern protocol version is unsupported",
                    )),
                    Some(-32020 | -32021) => {
                        Err(wire::protocol("MCP modern discovery request was rejected"))
                    }
                    _ => Err(Error::new(
                        ErrorKind::Unsupported,
                        "MCP legacy discovery probe rejected",
                    )),
                };
            }
            return Err(http_error(response.status));
        }
        let content_type = response
            .headers
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case("Content-Type"))
            .map(|(_, value)| value.as_str());
        let result = wire::response(&response.body, content_type, id)?;
        Ok((result, response))
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

fn parse_tool(value: &Value) -> Result<Tool, Error> {
    let name = value
        .get("name")
        .and_then(Value::as_str)
        .filter(|name| !name.is_empty())
        .ok_or_else(|| wire::protocol("MCP tool has no name"))?;
    let schema = value
        .get("inputSchema")
        .ok_or_else(|| wire::protocol("MCP tool has no input schema"))?;
    if !matches!(schema, Value::Object(_)) {
        return Err(wire::protocol("MCP tool input schema is not an object"));
    }
    let title = value
        .get("title")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let description = value
        .get("description")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    Ok(Tool {
        name: name.into(),
        title,
        description,
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

fn http_error(status: u16) -> Error {
    let mut error = Error::new(ErrorKind::Transport, format!("MCP HTTP {status}"));
    error.status = Some(status);
    error
}

#[cfg(test)]
mod tests;
