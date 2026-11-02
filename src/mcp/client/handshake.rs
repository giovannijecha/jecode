//! Protocol selection: a modern `server/discover` probe, else `initialize`.

use super::{Client, Mode, challenge, content_type, rejection, server_info};
use crate::cancel::Cancellation;
use crate::json::Value;
use crate::mcp::transport;
use crate::mcp::types::Error;
use crate::mcp::wire;

impl Client {
    pub(super) fn connect(&mut self, cancel: &Cancellation) -> Result<(), Error> {
        if self.mode.is_some() {
            return Ok(());
        }
        transport::validate_endpoint(&self.connection.endpoint)?;
        if !self.discover(cancel)? {
            self.initialize(cancel)?;
        }
        Ok(())
    }

    /// Returns false when the server should be reached through `initialize`.
    /// Only authentication, transport failures, cancellation and an explicit
    /// version refusal without a legacy offer stop the fallback.
    fn discover(&mut self, cancel: &Cancellation) -> Result<bool, Error> {
        let (id, response) = self.post(
            "server/discover",
            Value::object([]),
            None,
            Some(&Mode::Modern),
            &[],
            cancel,
        )?;
        let success = (200..300).contains(&response.status);
        if !success && let Some(error) = challenge(&response) {
            return Err(error);
        }
        match wire::decode(&response.body, content_type(&response), id) {
            Ok(wire::Reply::Result(result)) if success => {
                let modern = result
                    .get("supportedVersions")
                    .and_then(Value::as_array)
                    .is_some_and(|versions| {
                        versions
                            .iter()
                            .any(|version| version.as_str() == Some(wire::MODERN))
                    });
                if modern {
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
                    return Ok(true);
                }
            }
            Ok(wire::Reply::Error(error))
                if error.code == Some(-32022) && !error.offers_legacy() =>
            {
                return Err(error.into_error());
            }
            _ => {}
        }
        Ok(false)
    }

    pub(super) fn initialize(&mut self, cancel: &Cancellation) -> Result<(), Error> {
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
        let (id, response) = self.post("initialize", params, None, None, &[], cancel)?;
        let result = super::reply(&response, id)?;
        let version = result
            .get("protocolVersion")
            .and_then(Value::as_str)
            .ok_or_else(|| wire::protocol("MCP initialize response has no protocol version"))?;
        if !wire::legacy_version(version) {
            return Err(Error::new(
                crate::mcp::types::ErrorKind::Unsupported,
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
            return Err(rejection(&response, None));
        }
        self.mode = Some(mode);
        Ok(())
    }
}
