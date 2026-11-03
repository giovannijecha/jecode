//! OAuth public-client flows for explicitly configured MCP connections.
mod browser;
mod client;
mod discovery;
mod github;
mod identity;
pub(super) mod resource;
pub(super) mod scopes;
mod wire;

use super::types::{AuthMethod, ClientAuthMethod, Connection, Error, OAuthTokens};
use crate::{cancel::Cancellation, json::Value};
use std::time::{SystemTime, UNIX_EPOCH};

pub enum Update {
    Browser(String),
    DeviceCode { url: String, code: String },
    Waiting,
}

struct Probe {
    public: bool,
    challenge: Option<Vec<(String, String)>>,
}

pub fn methods(connection: &Connection, cancel: &Cancellation) -> Result<Vec<AuthMethod>, Error> {
    let probe = probe(connection, cancel)?;
    let mut result = if probe.public {
        vec![AuthMethod::None]
    } else {
        Vec::new()
    };
    if probe.challenge.is_some() {
        result.push(AuthMethod::Token);
    }
    if github::eligible(connection) {
        if !result.contains(&AuthMethod::Token) {
            result.push(AuthMethod::Token);
        }
        result.push(AuthMethod::GitHubDevice);
        return Ok(result);
    }
    match discovery::discover_optional(
        connection,
        cancel,
        probe.challenge.as_deref().unwrap_or(&[]),
    ) {
        Ok(Some(server)) => {
            if !result.contains(&AuthMethod::Token) {
                result.push(AuthMethod::Token);
            }
            let available = client::available(connection, &server);
            let same_binding = connection.oauth.as_ref().is_none_or(|tokens| {
                tokens.issuer == server.issuer && tokens.endpoint == connection.endpoint
            });
            if available && same_binding {
                result.push(AuthMethod::Browser);
            }
        }
        // Without usable OAuth metadata the server still offers None or Token.
        Err(error) if error.kind == super::types::ErrorKind::Cancelled => return Err(error),
        _ => {}
    }
    Ok(result)
}

fn probe(connection: &Connection, cancel: &Cancellation) -> Result<Probe, Error> {
    let modern = super::wire::request(1, "server/discover", Value::object([]), true).encode();
    let legacy = super::wire::request(
        2,
        "initialize",
        Value::object([
            ("protocolVersion", Value::string(super::wire::LEGACY)),
            ("capabilities", Value::object([])),
            (
                "clientInfo",
                Value::object([
                    ("name", Value::string("jecode")),
                    ("version", Value::string(env!("CARGO_PKG_VERSION"))),
                ]),
            ),
        ]),
        false,
    )
    .encode();
    for (index, body) in [modern, legacy].iter().enumerate() {
        let mut headers = vec![
            (
                "Accept".into(),
                "application/json, text/event-stream".into(),
            ),
            ("Content-Type".into(), "application/json".into()),
        ];
        if index == 0 {
            headers.push(("MCP-Protocol-Version".into(), super::wire::MODERN.into()));
            headers.push(("Mcp-Method".into(), "server/discover".into()));
        }
        headers.extend(connection.headers.iter().cloned());
        match super::transport::request_rpc(
            &connection.endpoint,
            "POST",
            &headers,
            Some(body.as_bytes()),
            connection.timeout_secs,
            cancel,
            index as u64 + 1,
        ) {
            Err(error) if error.kind == super::types::ErrorKind::Authentication => {
                return Ok(Probe {
                    public: false,
                    challenge: Some(error.headers),
                });
            }
            Err(error) => return Err(error),
            Ok(response) if (200..300).contains(&response.status) => {
                let challenge = catalog_probe(connection, cancel, index == 0)?;
                return Ok(Probe {
                    public: challenge.is_none(),
                    challenge,
                });
            }
            Ok(response)
                if index == 0 && matches!(response.status, 400 | 404 | 405 | 406 | 415) => {}
            Ok(_) => return Err(wire::failure("MCP endpoint could not be checked")),
        }
    }
    Err(wire::failure("MCP endpoint could not be checked"))
}

fn catalog_probe(
    connection: &Connection,
    cancel: &Cancellation,
    modern: bool,
) -> Result<Option<Vec<(String, String)>>, Error> {
    let body = super::wire::request(3, "tools/list", Value::object([]), modern).encode();
    let mut headers = vec![
        (
            "Accept".into(),
            "application/json, text/event-stream".into(),
        ),
        ("Content-Type".into(), "application/json".into()),
    ];
    if modern {
        headers.push(("MCP-Protocol-Version".into(), super::wire::MODERN.into()));
        headers.push(("Mcp-Method".into(), "tools/list".into()));
    } else {
        headers.push(("MCP-Protocol-Version".into(), super::wire::LEGACY.into()));
    }
    headers.extend(connection.headers.iter().cloned());
    match super::transport::request_rpc(
        &connection.endpoint,
        "POST",
        &headers,
        Some(body.as_bytes()),
        connection.timeout_secs,
        cancel,
        3,
    ) {
        Err(error) if error.kind == super::types::ErrorKind::Authentication => {
            Ok(Some(error.headers))
        }
        Err(error) if error.kind == super::types::ErrorKind::Cancelled => Err(error),
        _ => Ok(None),
    }
}

pub fn sign_in(
    connection: Connection,
    cancel: &Cancellation,
    emit: &mut dyn FnMut(Update) -> Result<(), String>,
) -> Result<Connection, Error> {
    match connection.method {
        AuthMethod::Browser => browser::sign_in(connection, cancel, emit),
        AuthMethod::GitHubDevice => github::sign_in(connection, cancel, emit),
        _ => Err(wire::failure(
            "Selected MCP method does not use interactive sign-in",
        )),
    }
}

pub fn refresh(connection: &Connection, cancel: &Cancellation) -> Result<Connection, Error> {
    let previous = connection
        .oauth
        .as_ref()
        .ok_or_else(|| wire::failure("MCP connection has no OAuth session"))?;
    let refresh = previous
        .refresh_token
        .as_deref()
        .ok_or_else(|| wire::failure("MCP OAuth session cannot be refreshed; sign in again"))?;
    if previous.github != (connection.method == AuthMethod::GitHubDevice) {
        return Err(wire::failure("MCP OAuth method changed; sign in again"));
    }
    if previous.github {
        if previous.issuer != "https://github.com"
            || previous.token_endpoint != "https://github.com/login/oauth/access_token"
            || previous.endpoint != connection.endpoint
            || !github::eligible(connection)
        {
            return Err(wire::failure("GitHub OAuth issuer changed; sign in again"));
        }
    } else {
        if previous.endpoint != connection.endpoint {
            return Err(wire::failure("MCP OAuth resource changed; sign in again"));
        }
        discovery::trusted_endpoint(&previous.token_endpoint, &previous.issuer)?;
        let challenge = probe(connection, cancel)?.challenge.unwrap_or_default();
        let discovered = discovery::discover_challenged(connection, cancel, &challenge)?;
        if discovered.issuer != previous.issuer
            || discovered.token != previous.token_endpoint
            || previous.resource.as_deref() != Some(discovered.resource.as_str())
        {
            return Err(wire::failure("MCP OAuth metadata changed; sign in again"));
        }
    }
    let mut fields = vec![("grant_type", "refresh_token"), ("refresh_token", refresh)];
    if !previous.github
        && let Some(resource) = &previous.resource
    {
        fields.push(("resource", resource));
    }
    let client = client::Client::from_tokens(previous);
    let value = client::token_request(&previous.token_endpoint, &fields, &client, cancel)?;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| wire::failure("System clock is invalid"))?
        .as_secs();
    let tokens = token_record(
        &value,
        TokenContext {
            endpoint: &previous.token_endpoint,
            bound_endpoint: &previous.endpoint,
            client: &previous.client_id,
            client_auth_method: previous.client_auth_method,
            client_secret: previous.client_secret.as_deref(),
            client_secret_expires_at: previous.client_secret_expires_at,
            issuer: &previous.issuer,
            resource: previous.resource.as_deref(),
            github: previous.github,
            now,
        },
        Some(previous),
    )?;
    let mut updated = connection.clone();
    updated.oauth = Some(tokens);
    super::config::validate(&updated)?;
    Ok(updated)
}

#[derive(Clone, Copy)]
struct TokenContext<'a> {
    endpoint: &'a str,
    bound_endpoint: &'a str,
    client: &'a str,
    client_auth_method: ClientAuthMethod,
    client_secret: Option<&'a str>,
    client_secret_expires_at: Option<u64>,
    issuer: &'a str,
    resource: Option<&'a str>,
    github: bool,
    now: u64,
}

fn token_record(
    value: &Value,
    ctx: TokenContext<'_>,
    previous: Option<&OAuthTokens>,
) -> Result<OAuthTokens, Error> {
    // GitHub answers some token errors with HTTP 200.
    if value
        .get("error")
        .is_some_and(|e| !matches!(e, Value::Null))
    {
        return Err(wire::rejection(200, Some(value)));
    }
    if value
        .get("token_type")
        .and_then(Value::as_str)
        .is_some_and(|kind| !kind.eq_ignore_ascii_case("bearer"))
    {
        return Err(wire::failure(
            "OAuth server returned an unsupported token type",
        ));
    }
    let access = wire::required(value, "access_token")?.to_owned();
    if access.len() > 16 * 1024 {
        return Err(wire::failure("OAuth access token is too large"));
    }
    let refresh = value
        .get("refresh_token")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .or_else(|| previous.and_then(|p| p.refresh_token.clone()));
    if ctx.github
        && previous.is_some()
        && value
            .get("refresh_token")
            .and_then(Value::as_str)
            .is_none_or(str::is_empty)
    {
        return Err(wire::failure(
            "GitHub refresh did not rotate the refresh token",
        ));
    }
    let lifetime = wire::seconds(value, "expires_in");
    if lifetime == Some(0) {
        return Err(wire::failure("OAuth server returned an expired token"));
    }
    let expires = lifetime.map(|s| ctx.now.saturating_add(s));
    Ok(OAuthTokens {
        access_token: access,
        refresh_token: refresh,
        expires_at: expires,
        token_endpoint: ctx.endpoint.into(),
        endpoint: ctx.bound_endpoint.into(),
        client_id: ctx.client.into(),
        client_auth_method: ctx.client_auth_method,
        client_secret: ctx.client_secret.map(str::to_owned),
        client_secret_expires_at: ctx.client_secret_expires_at,
        issuer: ctx.issuer.into(),
        scope: value
            .get("scope")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .or_else(|| previous.and_then(|p| p.scope.clone())),
        resource: ctx.resource.map(str::to_owned),
        github: ctx.github,
    })
}

#[cfg(test)]
mod live;
#[cfg(test)]
mod tests;
