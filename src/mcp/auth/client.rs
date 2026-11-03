//! OAuth client registration and token endpoint authentication.
use super::super::types::{ClientAuthMethod, Connection, Error, OAuthTokens};
use super::{discovery, identity, wire};
use crate::{cancel::Cancellation, json::Value};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone)]
pub(super) struct Client {
    pub id: String,
    pub method: ClientAuthMethod,
    pub secret: Option<String>,
    pub secret_expires_at: Option<u64>,
}

impl Client {
    pub fn public(id: &str) -> Self {
        Self {
            id: id.into(),
            method: ClientAuthMethod::None,
            secret: None,
            secret_expires_at: None,
        }
    }

    pub fn from_tokens(tokens: &OAuthTokens) -> Self {
        Self {
            id: tokens.client_id.clone(),
            method: tokens.client_auth_method,
            secret: tokens.client_secret.clone(),
            secret_expires_at: tokens.client_secret_expires_at,
        }
    }

    fn valid(&self) -> Result<(), Error> {
        if self.id.is_empty()
            || self.id.len() > 2048
            || self.id.bytes().any(|b| b <= 32 || b == 127)
        {
            return Err(wire::failure(
                "OAuth registration returned an invalid client ID",
            ));
        }
        match self.method {
            ClientAuthMethod::None if self.secret.is_some() || self.secret_expires_at.is_some() => {
                Err(wire::failure(
                    "OAuth registration returned inconsistent client credentials",
                ))
            }
            ClientAuthMethod::None => Ok(()),
            _ if self.secret.as_deref().is_none_or(str::is_empty) => Err(wire::failure(
                "OAuth registration did not return a client secret",
            )),
            _ if self.secret.as_ref().is_some_and(|s| {
                s.len() > 16 * 1024 || s.bytes().any(|b| b == b'\r' || b == b'\n' || b == 0)
            }) =>
            {
                Err(wire::failure(
                    "OAuth registration returned an invalid client secret",
                ))
            }
            _ => Ok(()),
        }
    }

    fn unexpired(&self) -> Result<(), Error> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| wire::failure("System clock is invalid"))?
            .as_secs();
        if self
            .secret_expires_at
            .is_some_and(|expires| expires != 0 && expires <= now)
        {
            return Err(wire::failure("OAuth client secret expired; sign in again"));
        }
        Ok(())
    }
}

pub(super) fn available(connection: &Connection, server: &discovery::Server) -> bool {
    let public = supports(server, "none");
    match connection.oauth_client_id.as_deref() {
        Some(id) if id.starts_with("https://") => server.cimd && public,
        Some(_) => public,
        None => {
            (server.cimd && public) || server.registration.is_some() && preferred(server).is_some()
        }
    }
}

fn supports(server: &discovery::Server, method: &str) -> bool {
    server
        .auth_methods
        .as_ref()
        .is_none_or(|methods| methods.iter().any(|value| value == method))
}

fn preferred(server: &discovery::Server) -> Option<&'static str> {
    if supports(server, "none") {
        Some("none")
    } else if supports(server, "client_secret_basic") {
        Some("client_secret_basic")
    } else if supports(server, "client_secret_post") {
        Some("client_secret_post")
    } else {
        None
    }
}

pub(super) fn resolve(
    connection: &Connection,
    server: &discovery::Server,
    redirect: &str,
    cancel: &Cancellation,
) -> Result<Client, Error> {
    if let Some(id) = connection.oauth_client_id.as_deref() {
        if !supports(server, "none") {
            return Err(wire::failure(
                "OAuth issuer does not support public client authentication",
            ));
        }
        if id.starts_with("https://") {
            if !server.cimd {
                return Err(wire::failure(
                    "OAuth issuer does not support Client ID Metadata Documents",
                ));
            }
            let value = wire::get_json(id, cancel)?
                .ok_or_else(|| wire::failure("Client ID Metadata Document was not found"))?;
            if wire::required(&value, "client_id")? != id {
                return Err(wire::failure(
                    "Client ID Metadata Document identity mismatch",
                ));
            }
            let redirects = value
                .get("redirect_uris")
                .and_then(Value::as_array)
                .ok_or_else(|| wire::failure("Client ID Metadata Document has no redirect URI"))?;
            if !redirects
                .iter()
                .filter_map(Value::as_str)
                .any(|uri| discovery::callback_matches(uri, redirect))
            {
                return Err(wire::failure(
                    "Client ID Metadata Document does not allow this callback",
                ));
            }
        }
        let client = Client::public(id);
        client.valid()?;
        return Ok(client);
    }
    if server.cimd && supports(server, "none") {
        let client = Client::public(identity::default_client_id());
        client.valid()?;
        return Ok(client);
    }
    let registration = server
        .registration
        .as_deref()
        .ok_or_else(|| wire::failure("OAuth issuer requires a pre-registered client ID"))?;
    let requested = preferred(server).ok_or_else(|| {
        wire::failure("OAuth issuer does not support a compatible client authentication method")
    })?;
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
        ("token_endpoint_auth_method", Value::string(requested)),
        ("application_type", Value::string("native")),
    ]);
    let response = wire::post_json(registration, &body, cancel)?;
    registered(&response, requested)
}

fn registered(response: &Value, requested: &str) -> Result<Client, Error> {
    let id = wire::required(response, "client_id")?.to_owned();
    let secret = match response.get("client_secret") {
        None | Some(Value::Null) => None,
        Some(Value::String(secret)) if secret.is_empty() => None,
        Some(Value::String(secret)) => Some(secret.clone()),
        _ => {
            return Err(wire::failure(
                "OAuth registration returned an invalid client secret",
            ));
        }
    };
    let method = match response.get("token_endpoint_auth_method") {
        None if secret.is_none() && requested == "none" => ClientAuthMethod::None,
        None => ClientAuthMethod::ClientSecretBasic, // RFC 7591 default.
        Some(Value::String(method)) if method == "none" => ClientAuthMethod::None,
        Some(Value::String(method)) if method == "client_secret_basic" => {
            ClientAuthMethod::ClientSecretBasic
        }
        Some(Value::String(method)) if method == "client_secret_post" => {
            ClientAuthMethod::ClientSecretPost
        }
        _ => {
            return Err(wire::failure(
                "OAuth registration returned an unsupported client authentication method",
            ));
        }
    };
    let secret_expires_at = match response.get("client_secret_expires_at") {
        None | Some(Value::Null) => None,
        Some(_) => Some(
            wire::seconds(response, "client_secret_expires_at").ok_or_else(|| {
                wire::failure("OAuth registration returned an invalid client secret expiry")
            })?,
        ),
    };
    let client = Client {
        id,
        method,
        // A server may issue a secret even when it selects public-client auth.
        // Only the selected authentication method determines what we retain.
        secret: if method == ClientAuthMethod::None {
            None
        } else {
            secret
        },
        secret_expires_at: if method == ClientAuthMethod::None {
            None
        } else {
            secret_expires_at
        },
    };
    client.valid()?;
    client.unexpired()?;
    Ok(client)
}

pub(super) fn token_request(
    endpoint: &str,
    fields: &[(&str, &str)],
    client: &Client,
    cancel: &Cancellation,
) -> Result<Value, Error> {
    client.valid()?;
    client.unexpired()?;
    let mut body = fields.to_vec();
    let mut headers = Vec::new();
    match client.method {
        ClientAuthMethod::None => body.push(("client_id", &client.id)),
        ClientAuthMethod::ClientSecretPost => {
            body.push(("client_id", &client.id));
            body.push(("client_secret", client.secret.as_deref().unwrap()));
        }
        ClientAuthMethod::ClientSecretBasic => {
            let credentials = format!(
                "{}:{}",
                wire::encode(&client.id),
                wire::encode(client.secret.as_deref().unwrap())
            );
            headers.push((
                "Authorization".into(),
                format!(
                    "Basic {}",
                    crate::attachments::base64::encode(credentials.as_bytes())
                ),
            ));
        }
    }
    wire::post_form_with_headers(endpoint, &body, &headers, cancel)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::json;

    #[test]
    fn registration_response_selects_actual_method_and_checks_secret() {
        let parse = |body, requested| registered(&json::parse(body).unwrap(), requested);
        assert_eq!(
            parse(r#"{"client_id":"public"}"#, "none").unwrap().method,
            ClientAuthMethod::None
        );
        assert_eq!(
            parse(
                r#"{"client_id":"public","client_secret":"","token_endpoint_auth_method":"none"}"#,
                "none"
            )
            .unwrap()
            .method,
            ClientAuthMethod::None
        );
        assert_eq!(
            parse(
                r#"{"client_id":"issued","client_secret":"secret","client_secret_expires_at":0}"#,
                "none"
            )
            .unwrap()
            .method,
            ClientAuthMethod::ClientSecretBasic
        );
        let public = parse(
            r#"{"client_id":"public","client_secret":"unused-secret","client_secret_expires_at":1,"token_endpoint_auth_method":"none"}"#,
            "none",
        ).unwrap();
        assert_eq!(public.method, ClientAuthMethod::None);
        assert!(public.secret.is_none());
        assert!(public.secret_expires_at.is_none());
        for body in [
            r#"{"client_id":"issued","token_endpoint_auth_method":"client_secret_basic"}"#,
            r#"{"client_id":"issued","token_endpoint_auth_method":"client_secret_post"}"#,
            r#"{"client_id":"issued","client_secret":"secret","token_endpoint_auth_method":"private_key_jwt"}"#,
            r#"{"client_id":"issued","client_secret":"secret","client_secret_expires_at":"bad"}"#,
            r#"{"client_id":"issued","client_secret":"secret","client_secret_expires_at":1}"#,
        ] {
            assert!(parse(body, "none").is_err(), "{body}");
        }
        let expired = Client {
            id: "issued".into(),
            method: ClientAuthMethod::ClientSecretBasic,
            secret: Some("secret".into()),
            secret_expires_at: Some(1),
        };
        let error = token_request(
            "https://issuer.test/token",
            &[("grant_type", "refresh_token")],
            &expired,
            &Cancellation::default(),
        )
        .map(drop)
        .unwrap_err();
        assert!(error.message.contains("secret expired"));
    }
}
