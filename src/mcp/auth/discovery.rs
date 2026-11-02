use super::super::types::Error;
use super::super::{config, types::Connection};
use super::wire::{self, failure, required};
use crate::{cancel::Cancellation, json::Value};

pub(super) struct Server {
    pub issuer: String,
    pub authorize: String,
    pub token: String,
    pub registration: Option<String>,
    pub resource: String,
    pub scope: Option<String>,
    pub cimd: bool,
    pub response_iss_required: bool,
}

pub(super) fn discover_challenged(
    connection: &Connection,
    cancel: &Cancellation,
    challenge: &[(String, String)],
) -> Result<Server, Error> {
    discover_optional(connection, cancel, challenge)?
        .ok_or_else(|| failure("MCP server did not publish protected-resource metadata"))
}

pub(super) fn discover_optional(
    connection: &Connection,
    cancel: &Cancellation,
    challenge: &[(String, String)],
) -> Result<Option<Server>, Error> {
    let endpoint = &connection.endpoint;
    let base = wire::origin(endpoint)?;
    let path = endpoint
        .strip_prefix(&base)
        .unwrap_or("")
        .split('?')
        .next()
        .unwrap_or("");
    let mut candidates = if path.is_empty() || path == "/" {
        vec![format!("{base}/.well-known/oauth-protected-resource")]
    } else {
        vec![
            format!("{base}/.well-known/oauth-protected-resource{path}"),
            format!("{base}/.well-known/oauth-protected-resource"),
        ]
    };
    if let Some(url) = wire::challenge_param(challenge, "resource_metadata") {
        config::validate_url(&url)?;
        candidates.insert(0, url);
    }
    let Some(metadata) = wire::first_json(&candidates, cancel)? else {
        return Ok(None);
    };
    let announced = required(&metadata, "resource")?;
    if announced != endpoint && announced != endpoint.split('?').next().unwrap_or(endpoint) {
        return Err(failure(
            "MCP protected-resource identity does not match the configured endpoint",
        ));
    }
    let issuers = metadata
        .get("authorization_servers")
        .and_then(Value::as_array)
        .ok_or_else(|| failure("MCP server did not announce an authorization server"))?;
    let issuer = issuers
        .first()
        .and_then(Value::as_str)
        .ok_or_else(|| failure("MCP authorization server is invalid"))?;
    config::validate_url(issuer)?;
    let base = wire::origin(issuer)?;
    let suffix = issuer
        .strip_prefix(&base)
        .unwrap_or("")
        .trim_end_matches('/');
    let mut urls = vec![
        format!("{base}/.well-known/oauth-authorization-server{suffix}"),
        format!("{base}/.well-known/openid-configuration{suffix}"),
    ];
    if !suffix.is_empty() {
        urls.push(format!("{issuer}/.well-known/openid-configuration"));
    }
    let authorization = wire::first_json(&urls, cancel)?
        .ok_or_else(|| failure("OAuth authorization server metadata was not found"))?;
    if required(&authorization, "issuer")? != issuer {
        return Err(failure(
            "OAuth issuer metadata does not match the announced issuer",
        ));
    }
    pkce(&authorization)?;
    let authorize = required(&authorization, "authorization_endpoint")?.to_owned();
    let token = required(&authorization, "token_endpoint")?.to_owned();
    trusted_endpoint(&authorize, issuer)?;
    trusted_endpoint(&token, issuer)?;
    let registration = authorization
        .get("registration_endpoint")
        .and_then(Value::as_str)
        .map(str::to_owned);
    if let Some(url) = &registration {
        trusted_endpoint(url, issuer)?;
    }
    let scope = wire::challenge_param(challenge, "scope")
        .filter(|s| !s.is_empty())
        .or_else(|| {
            metadata
                .get("scopes_supported")
                .and_then(Value::as_array)
                .map(|values| {
                    values
                        .iter()
                        .filter_map(Value::as_str)
                        .collect::<Vec<_>>()
                        .join(" ")
                })
                .filter(|s| !s.is_empty())
        });
    Ok(Some(Server {
        issuer: issuer.into(),
        authorize,
        token,
        registration,
        resource: announced.into(),
        scope,
        cimd: matches!(
            authorization.get("client_id_metadata_document_supported"),
            Some(Value::Bool(true))
        ),
        response_iss_required: matches!(
            authorization.get("authorization_response_iss_parameter_supported"),
            Some(Value::Bool(true))
        ),
    }))
}

/// Servers often omit the method list; only an explicit list without S256 is refused.
pub(super) fn pkce(authorization: &Value) -> Result<(), Error> {
    match authorization.get("code_challenge_methods_supported") {
        Some(methods)
            if !methods
                .as_array()
                .is_some_and(|list| list.iter().any(|m| m.as_str() == Some("S256"))) =>
        {
            Err(failure(
                "OAuth authorization server does not support PKCE S256",
            ))
        }
        _ => Ok(()),
    }
}

pub(super) fn trusted_endpoint(url: &str, issuer: &str) -> Result<(), Error> {
    config::validate_url(url)?;
    if issuer.starts_with("https://") && !url.starts_with("https://") {
        return Err(failure("OAuth endpoint must use HTTPS"));
    }
    Ok(())
}

pub(super) fn client_id(
    connection: &Connection,
    server: &Server,
    redirect: &str,
    cancel: &Cancellation,
) -> Result<String, Error> {
    // A registered client is bound to this sign-in's redirect; only refresh reuses it.
    if let Some(id) = connection.oauth_client_id.as_deref() {
        if id.starts_with("https://") {
            if !server.cimd {
                return Err(failure(
                    "OAuth issuer does not support Client ID Metadata Documents",
                ));
            }
            let value = wire::get_json(id, cancel)?
                .ok_or_else(|| failure("Client ID Metadata Document was not found"))?;
            if required(&value, "client_id")? != id {
                return Err(failure("Client ID Metadata Document identity mismatch"));
            }
            let redirects = value
                .get("redirect_uris")
                .and_then(Value::as_array)
                .ok_or_else(|| failure("Client ID Metadata Document has no redirect URI"))?;
            if !redirects
                .iter()
                .filter_map(Value::as_str)
                .any(|uri| callback_matches(uri, redirect))
            {
                return Err(failure(
                    "Client ID Metadata Document does not allow this callback",
                ));
            }
        }
        return Ok(id.into());
    }
    let registration = server
        .registration
        .as_ref()
        .ok_or_else(|| failure("OAuth issuer requires a pre-registered client ID"))?;
    let body = Value::object([
        ("client_name", Value::string("Jecode")),
        ("redirect_uris", Value::Array(vec![Value::string(redirect)])),
        (
            "grant_types",
            Value::Array(vec![
                Value::string("authorization_code"),
                Value::string("refresh_token"),
            ]),
        ),
        ("response_types", Value::Array(vec![Value::string("code")])),
        ("token_endpoint_auth_method", Value::string("none")),
        ("application_type", Value::string("native")),
    ]);
    let response = wire::post_json(registration, &body, cancel)?;
    let method = response
        .get("token_endpoint_auth_method")
        .and_then(Value::as_str);
    if method.is_some_and(|method| method != "none")
        || method != Some("none")
            && response
                .get("client_secret")
                .is_some_and(|value| !matches!(value, Value::Null) && value.as_str() != Some(""))
    {
        return Err(failure("OAuth registration required a confidential client"));
    }
    Ok(required(&response, "client_id")?.into())
}

fn callback_matches(registered: &str, actual: &str) -> bool {
    // Native OAuth loopback redirects may vary only their ephemeral port (RFC 8252).
    fn path(uri: &str) -> Option<&str> {
        let rest = uri.strip_prefix("http://127.0.0.1")?;
        if let Some(path) = rest.strip_prefix('/') {
            return Some(path);
        }
        let (port, path) = rest.strip_prefix(':')?.split_once('/')?;
        port.parse::<u16>().ok().filter(|port| *port > 0)?;
        Some(path)
    }
    registered == actual
        || path(registered)
            .zip(path(actual))
            .is_some_and(|(a, b)| a == b)
}

#[cfg(test)]
mod tests {
    use super::{callback_matches, pkce};
    use crate::json;

    #[test]
    fn pkce_is_refused_only_when_metadata_lists_methods_without_s256() {
        for (metadata, ok) in [
            (r#"{}"#, true),
            (
                r#"{"code_challenge_methods_supported":["plain","S256"]}"#,
                true,
            ),
            (r#"{"code_challenge_methods_supported":["plain"]}"#, false),
            (r#"{"code_challenge_methods_supported":"S256"}"#, false),
        ] {
            assert_eq!(
                pkce(&json::parse(metadata).unwrap()).is_ok(),
                ok,
                "{metadata}"
            );
        }
    }

    #[test]
    fn native_oauth_metadata_accepts_ephemeral_ports_only_for_the_same_loopback_path() {
        let actual = "http://127.0.0.1:41234/callback";
        assert!(callback_matches("http://127.0.0.1/callback", actual));
        assert!(callback_matches("http://127.0.0.1:8080/callback", actual));
        for uri in [
            "http://127.0.0.1.example/callback",
            "http://127.0.0.1/other",
            "https://example.test/callback",
            "http://127.0.0.1:0/callback",
        ] {
            assert!(!callback_matches(uri, actual), "{uri}");
        }
    }
}
