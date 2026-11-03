use super::super::types::Error;
use super::super::{config, types::Connection};
use super::resource;
use super::wire::{self, failure, required};
use crate::{cancel::Cancellation, json::Value};

pub(super) struct Server {
    pub issuer: String,
    pub authorize: String,
    pub token: String,
    pub registration: Option<String>,
    pub resource: String,
    pub scope: Option<String>,
    pub offline_access: bool,
    pub cimd: bool,
    pub response_iss_required: bool,
    pub auth_methods: Option<Vec<String>>,
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
    let Some((metadata, announced, issuer)) =
        wire::first_valid_json(&candidates, cancel, |metadata| {
            let announced = required(&metadata, "resource")?.to_owned();
            if !resource::matches_endpoint(&announced, endpoint)
                && !resource::matches_endpoint(
                    &announced,
                    endpoint.split('?').next().unwrap_or(endpoint),
                )
            {
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
                .ok_or_else(|| failure("MCP authorization server is invalid"))?
                .to_owned();
            config::validate_url(&issuer)?;
            Ok((metadata, announced, issuer))
        })?
    else {
        return Ok(None);
    };
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
    let scope = super::scopes::union(&[scope.as_deref()])?;
    let base = wire::origin(&issuer)?;
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
    let server = wire::first_valid_json(&urls, cancel, |authorization| {
        if required(&authorization, "issuer")? != issuer.as_str() {
            return Err(failure(
                "OAuth issuer metadata does not match the announced issuer",
            ));
        }
        pkce(&authorization)?;
        let authorize = required(&authorization, "authorization_endpoint")?.to_owned();
        let token = required(&authorization, "token_endpoint")?.to_owned();
        trusted_endpoint(&authorize, &issuer)?;
        trusted_endpoint(&token, &issuer)?;
        let registration = authorization
            .get("registration_endpoint")
            .and_then(Value::as_str)
            .map(str::to_owned);
        if let Some(url) = &registration {
            trusted_endpoint(url, &issuer)?;
        }
        let auth_methods = match authorization.get("token_endpoint_auth_methods_supported") {
            None => None,
            Some(Value::Array(methods)) => Some(
                methods
                    .iter()
                    .map(|method| {
                        method.as_str().map(str::to_owned).ok_or_else(|| {
                            failure("OAuth token authentication metadata is invalid")
                        })
                    })
                    .collect::<Result<Vec<_>, _>>()?,
            ),
            _ => return Err(failure("OAuth token authentication metadata is invalid")),
        };
        Ok(Server {
            issuer: issuer.clone(),
            authorize,
            token,
            registration,
            resource: announced.clone(),
            scope: scope.clone(),
            offline_access: supports_offline_access(&authorization),
            cimd: matches!(
                authorization.get("client_id_metadata_document_supported"),
                Some(Value::Bool(true))
            ),
            response_iss_required: matches!(
                authorization.get("authorization_response_iss_parameter_supported"),
                Some(Value::Bool(true))
            ),
            auth_methods,
        })
    })?
    .ok_or_else(|| failure("OAuth authorization server metadata was not found"))?;
    Ok(Some(server))
}

fn supports_offline_access(metadata: &Value) -> bool {
    metadata
        .get("scopes_supported")
        .and_then(Value::as_array)
        .is_some_and(|values| {
            values
                .iter()
                .any(|value| value.as_str() == Some("offline_access"))
        })
}

/// MCP authorization requires advertised PKCE S256 support before sign-in.
pub(super) fn pkce(authorization: &Value) -> Result<(), Error> {
    if authorization
        .get("code_challenge_methods_supported")
        .and_then(Value::as_array)
        .is_some_and(|list| list.iter().any(|m| m.as_str() == Some("S256")))
    {
        Ok(())
    } else {
        Err(failure(
            "OAuth authorization server does not advertise PKCE S256 support",
        ))
    }
}

pub(super) fn trusted_endpoint(url: &str, issuer: &str) -> Result<(), Error> {
    config::validate_url(url)?;
    if issuer.starts_with("https://") && !url.starts_with("https://") {
        return Err(failure("OAuth endpoint must use HTTPS"));
    }
    Ok(())
}

pub(super) fn callback_matches(registered: &str, actual: &str) -> bool {
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
    use super::{callback_matches, pkce, supports_offline_access};
    use crate::json;

    #[test]
    fn pkce_requires_advertised_s256_support() {
        for (metadata, ok) in [
            (r#"{}"#, false),
            (
                r#"{"code_challenge_methods_supported":["plain","S256"]}"#,
                true,
            ),
            (r#"{"code_challenge_methods_supported":["plain"]}"#, false),
            (r#"{"code_challenge_methods_supported":[]}"#, false),
            (r#"{"code_challenge_methods_supported":null}"#, false),
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
    fn detects_advertised_offline_access() {
        for (metadata, expected) in [
            (r#"{}"#, false),
            (r#"{"scopes_supported":["openid"]}"#, false),
            (r#"{"scopes_supported":["openid","offline_access"]}"#, true),
        ] {
            assert_eq!(
                supports_offline_access(&json::parse(metadata).unwrap()),
                expected
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
