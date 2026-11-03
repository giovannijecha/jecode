//! MCP connection validation and its independent on-disk representation.
use crate::json::Value;

use super::types::{AuthMethod, ClientAuthMethod, Connection, Error, ErrorKind, OAuthTokens};

fn invalid(message: &str) -> Error {
    Error::new(ErrorKind::Configuration, message)
}

pub fn validate_url(url: &str) -> Result<(), Error> {
    if url.len() > 2048 || url.bytes().any(|b| b <= 32 || b == 127) || url.contains('#') {
        return Err(invalid("MCP URL is invalid"));
    }
    let (secure, rest) = if let Some(rest) = url.strip_prefix("https://") {
        (true, rest)
    } else if let Some(rest) = url.strip_prefix("http://") {
        (false, rest)
    } else {
        return Err(invalid("MCP URL must use HTTPS or loopback HTTP"));
    };
    let authority = rest.split(['/', '?']).next().unwrap_or("");
    if authority.is_empty()
        || authority.contains('@')
        || authority.contains('\\')
        || authority.contains('%')
        || authority.ends_with('.')
    {
        return Err(invalid("MCP URL has an invalid host"));
    }
    let (host, port) = if let Some(rest) = authority.strip_prefix('[') {
        let (address, suffix) = rest
            .split_once(']')
            .ok_or_else(|| invalid("MCP URL has an invalid host"))?;
        let ip = address
            .parse::<std::net::Ipv6Addr>()
            .map_err(|_| invalid("MCP URL has an invalid host"))?;
        if !secure && !ip.is_loopback() {
            return Err(invalid("MCP HTTP is allowed only on loopback"));
        }
        if !suffix.is_empty() && !suffix.starts_with(':') {
            return Err(invalid("MCP URL has an invalid port"));
        }
        (address, suffix.strip_prefix(':'))
    } else {
        let (host, port) = authority
            .split_once(':')
            .map_or((authority, None), |(host, port)| (host, Some(port)));
        if host.is_empty()
            || !host
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-')
        {
            return Err(invalid("MCP URL has an invalid host"));
        }
        (host, port)
    };
    if !secure
        && !host.eq_ignore_ascii_case("localhost")
        && !host
            .parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback())
    {
        return Err(invalid("MCP HTTP is allowed only on loopback"));
    }
    if port.is_some_and(|port| port.parse::<u16>().ok().is_none_or(|n| n == 0)) {
        return Err(invalid("MCP URL has an invalid port"));
    }
    Ok(())
}

/// The GitHub MCP endpoint, with or without its trailing slash.
pub fn github_endpoint(url: &str) -> bool {
    url.strip_prefix("https://api.githubcopilot.com/mcp")
        .is_some_and(|rest| rest.is_empty() || rest.starts_with(['/', '?']))
}

pub fn validate(connection: &Connection) -> Result<(), Error> {
    if connection.name.trim().is_empty()
        || connection.name.trim() != connection.name
        || connection.name.len() > 128
        || connection.name.chars().any(char::is_control)
    {
        return Err(invalid("MCP name must be 1-128 printable characters"));
    }
    validate_url(&connection.endpoint)?;
    if !connection.id.is_empty()
        && (connection.id.len() > 80
            || !connection
                .id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-'))
    {
        return Err(invalid("MCP connection ID is invalid"));
    }
    if !(1..=600).contains(&connection.timeout_secs) {
        return Err(invalid("MCP timeout must be 1-600 seconds"));
    }
    if connection.token.len() > 16 * 1024
        || connection
            .token
            .bytes()
            .any(|b| b == b'\r' || b == b'\n' || b == 0)
        || connection.oauth_client_id.as_ref().is_some_and(|id| {
            id.len() > 2048 || id.is_empty() || id.bytes().any(|b| b <= 32 || b == 127)
        })
    {
        return Err(invalid("MCP credential exceeds the supported size"));
    }
    if connection.method == AuthMethod::Token && connection.token.is_empty() {
        return Err(invalid("MCP bearer token is missing"));
    }
    if connection.method != AuthMethod::Token && !connection.token.is_empty() {
        return Err(invalid(
            "MCP bearer token does not match the selected method",
        ));
    }
    if connection.pending_scope.is_some()
        && !matches!(
            super::auth::scopes::union(&[connection.pending_scope.as_deref()]),
            Ok(Some(_))
        )
    {
        return Err(invalid("MCP pending OAuth scope is invalid"));
    }
    if github_endpoint(&connection.endpoint)
        && connection
            .oauth_client_id
            .as_ref()
            .is_some_and(|id| !id.bytes().all(|b| b.is_ascii_graphic()))
    {
        return Err(invalid("GitHub OAuth client ID must be printable ASCII"));
    }
    if connection.headers.len() > 32 {
        return Err(invalid("MCP has too many custom headers"));
    }
    for (name, value) in &connection.headers {
        if name.is_empty()
            || name.len() > 128
            || !name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b))
            || value.len() > 16 * 1024
            || value.bytes().any(|b| b == b'\r' || b == b'\n' || b == 0)
            || name.to_ascii_lowercase().starts_with("mcp-")
            || matches!(
                name.to_ascii_lowercase().as_str(),
                "authorization"
                    | "proxy-authorization"
                    | "host"
                    | "content-length"
                    | "transfer-encoding"
                    | "te"
                    | "connection"
                    | "upgrade"
                    | "expect"
                    | "accept"
                    | "content-type"
                    | "last-event-id"
            )
        {
            return Err(invalid("MCP custom header is invalid or reserved"));
        }
    }
    if connection.oauth.is_some()
        && !matches!(
            connection.method,
            AuthMethod::Browser | AuthMethod::GitHubDevice
        )
    {
        return Err(invalid(
            "MCP OAuth credentials do not match the selected method",
        ));
    }
    if let Some(tokens) = &connection.oauth {
        if tokens.github != (connection.method == AuthMethod::GitHubDevice) {
            return Err(invalid(
                "MCP OAuth session does not match the selected provider",
            ));
        }
        if tokens.github
            && (tokens.issuer != "https://github.com"
                || tokens.token_endpoint != "https://github.com/login/oauth/access_token"
                || !github_endpoint(&connection.endpoint))
        {
            return Err(invalid("GitHub OAuth endpoint is invalid"));
        }
        if tokens.endpoint != connection.endpoint {
            return Err(invalid("MCP OAuth session belongs to a different endpoint"));
        }
        if tokens.resource.as_deref().is_none_or(|resource| {
            !super::auth::resource::matches_endpoint(resource, &connection.endpoint)
                && !super::auth::resource::matches_endpoint(
                    resource,
                    connection
                        .endpoint
                        .split('?')
                        .next()
                        .unwrap_or(&connection.endpoint),
                )
        }) {
            return Err(invalid("MCP OAuth session belongs to a different endpoint"));
        }
        if tokens.access_token.is_empty()
            || tokens.access_token.len() > 16 * 1024
            || tokens
                .access_token
                .bytes()
                .any(|b| b == b'\r' || b == b'\n' || b == 0)
            || tokens.refresh_token.as_ref().is_some_and(|s| {
                s.len() > 16 * 1024 || s.bytes().any(|b| b == b'\r' || b == b'\n' || b == 0)
            })
            || tokens.client_id.is_empty()
            || tokens.client_id.len() > 2048
            || tokens.client_id.bytes().any(|b| b <= 32 || b == 127)
            || tokens.issuer.is_empty()
            || tokens.client_secret.as_ref().is_some_and(|s| {
                s.is_empty()
                    || s.len() > 16 * 1024
                    || s.bytes().any(|b| b == b'\r' || b == b'\n' || b == 0)
            })
            || match tokens.client_auth_method {
                ClientAuthMethod::None => {
                    tokens.client_secret.is_some() || tokens.client_secret_expires_at.is_some()
                }
                ClientAuthMethod::ClientSecretBasic | ClientAuthMethod::ClientSecretPost => {
                    tokens.client_secret.is_none()
                }
            }
            || tokens.github && tokens.client_auth_method != ClientAuthMethod::None
        {
            return Err(invalid("MCP OAuth record is invalid"));
        }
        validate_url(&tokens.token_endpoint)?;
        validate_url(&tokens.issuer)?;
        if let Some(resource) = &tokens.resource {
            validate_url(resource)?;
        }
    }
    Ok(())
}

fn field<'a>(value: &'a Value, key: &str) -> Result<&'a str, Error> {
    value
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| invalid("MCP connection file has a missing or invalid field"))
}
fn optional(value: &Value, key: &str) -> Result<Option<String>, Error> {
    match value.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s.clone())),
        _ => Err(invalid("MCP connection file has an invalid field")),
    }
}
fn number(value: &Value, key: &str) -> Result<Option<u64>, Error> {
    match value.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Number(s)) => s
            .parse()
            .map(Some)
            .map_err(|_| invalid("MCP connection file has an invalid number")),
        _ => Err(invalid("MCP connection file has an invalid number")),
    }
}
fn method_name(method: AuthMethod) -> &'static str {
    match method {
        AuthMethod::None => "none",
        AuthMethod::Token => "token",
        AuthMethod::Browser => "browser",
        AuthMethod::GitHubDevice => "github_device",
    }
}
pub fn read(value: &Value) -> Result<Connection, Error> {
    let method = match field(value, "method")? {
        "none" => AuthMethod::None,
        "token" => AuthMethod::Token,
        "browser" => AuthMethod::Browser,
        "github_device" => AuthMethod::GitHubDevice,
        _ => return Err(invalid("MCP authentication method is unknown")),
    };
    let enabled = match value.get("enabled") {
        Some(Value::Bool(b)) => *b,
        None => true,
        _ => return Err(invalid("MCP enabled flag is invalid")),
    };
    let headers = match value.get("headers") {
        None => Vec::new(),
        Some(Value::Object(map)) => map
            .iter()
            .map(|(k, v)| {
                v.as_str()
                    .map(|s| (k.clone(), s.to_owned()))
                    .ok_or_else(|| invalid("MCP header is invalid"))
            })
            .collect::<Result<Vec<_>, _>>()?,
        _ => return Err(invalid("MCP headers must be an object")),
    };
    let oauth = match value.get("oauth") {
        None | Some(Value::Null) => None,
        Some(v @ Value::Object(_)) => Some(OAuthTokens {
            access_token: field(v, "access_token")?.into(),
            refresh_token: optional(v, "refresh_token")?,
            expires_at: number(v, "expires_at")?,
            token_endpoint: field(v, "token_endpoint")?.into(),
            endpoint: field(v, "endpoint")?.into(),
            client_id: field(v, "client_id")?.into(),
            client_auth_method: match optional(v, "client_auth_method")?.as_deref() {
                None | Some("none") => ClientAuthMethod::None,
                Some("client_secret_basic") => ClientAuthMethod::ClientSecretBasic,
                Some("client_secret_post") => ClientAuthMethod::ClientSecretPost,
                _ => return Err(invalid("MCP OAuth client authentication method is invalid")),
            },
            client_secret: optional(v, "client_secret")?,
            client_secret_expires_at: number(v, "client_secret_expires_at")?,
            issuer: field(v, "issuer")?.into(),
            scope: optional(v, "scope")?,
            resource: optional(v, "resource")?,
            github: match v.get("github") {
                Some(Value::Bool(value)) => *value,
                _ => return Err(invalid("MCP OAuth record has an invalid provider flag")),
            },
        }),
        _ => return Err(invalid("MCP OAuth record is invalid")),
    };
    let connection = Connection {
        id: field(value, "id")?.into(),
        name: field(value, "name")?.into(),
        endpoint: field(value, "endpoint")?.into(),
        enabled,
        method,
        token: optional(value, "token")?.unwrap_or_default(),
        oauth,
        oauth_client_id: optional(value, "oauth_client_id")?,
        pending_scope: optional(value, "pending_scope")?,
        timeout_secs: number(value, "timeout_secs")?.unwrap_or(60),
        headers,
    };
    validate(&connection)?;
    Ok(connection)
}

pub fn write(connection: &Connection, previous: Option<Value>) -> Value {
    let mut value = previous.unwrap_or_else(|| Value::object([]));
    let Value::Object(ref mut map) = value else {
        unreachable!()
    };
    map.insert("id".into(), Value::string(&connection.id));
    map.insert("name".into(), Value::string(&connection.name));
    map.insert("endpoint".into(), Value::string(&connection.endpoint));
    map.insert("enabled".into(), Value::Bool(connection.enabled));
    map.insert(
        "method".into(),
        Value::string(method_name(connection.method)),
    );
    map.insert("token".into(), Value::string(&connection.token));
    map.insert(
        "timeout_secs".into(),
        Value::number(connection.timeout_secs),
    );
    map.insert(
        "headers".into(),
        Value::Object(
            connection
                .headers
                .iter()
                .cloned()
                .map(|(k, v)| (k, Value::string(v)))
                .collect(),
        ),
    );
    if let Some(id) = &connection.oauth_client_id {
        map.insert("oauth_client_id".into(), Value::string(id));
    } else {
        map.remove("oauth_client_id");
    }
    if let Some(scope) = &connection.pending_scope {
        map.insert("pending_scope".into(), Value::string(scope));
    } else {
        map.remove("pending_scope");
    }
    if let Some(tokens) = &connection.oauth {
        let mut o = map
            .get("oauth")
            .filter(|v| matches!(v, Value::Object(_)))
            .cloned()
            .unwrap_or_else(|| Value::object([]));
        let Value::Object(ref mut fields) = o else {
            unreachable!()
        };
        fields.insert("access_token".into(), Value::string(&tokens.access_token));
        fields.insert(
            "refresh_token".into(),
            tokens
                .refresh_token
                .as_ref()
                .map_or(Value::Null, Value::string),
        );
        fields.insert(
            "expires_at".into(),
            tokens.expires_at.map_or(Value::Null, Value::number),
        );
        fields.insert(
            "token_endpoint".into(),
            Value::string(&tokens.token_endpoint),
        );
        fields.insert("endpoint".into(), Value::string(&tokens.endpoint));
        fields.insert("client_id".into(), Value::string(&tokens.client_id));
        fields.insert(
            "client_auth_method".into(),
            Value::string(match tokens.client_auth_method {
                ClientAuthMethod::None => "none",
                ClientAuthMethod::ClientSecretBasic => "client_secret_basic",
                ClientAuthMethod::ClientSecretPost => "client_secret_post",
            }),
        );
        if let Some(secret) = &tokens.client_secret {
            fields.insert("client_secret".into(), Value::string(secret));
        } else {
            fields.remove("client_secret");
        }
        if let Some(expires_at) = tokens.client_secret_expires_at {
            fields.insert("client_secret_expires_at".into(), Value::number(expires_at));
        } else {
            fields.remove("client_secret_expires_at");
        }
        fields.insert("issuer".into(), Value::string(&tokens.issuer));
        fields.insert(
            "scope".into(),
            tokens.scope.as_ref().map_or(Value::Null, Value::string),
        );
        fields.insert(
            "resource".into(),
            tokens.resource.as_ref().map_or(Value::Null, Value::string),
        );
        fields.insert("github".into(), Value::Bool(tokens.github));
        map.insert("oauth".into(), o);
    } else {
        map.remove("oauth");
    }
    value
}
