use super::{native, wire};
use crate::cancel::Cancellation;
use crate::json::{self, Value};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug)]
pub(super) struct Claims {
    pub subject: String,
    pub email: String,
    pub label: String,
}

fn text<'a>(value: &'a Value, key: &str) -> Result<&'a str, String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("ID token has no valid {key}"))
}

fn number(value: &Value, key: &str) -> Result<u64, String> {
    match value.get(key) {
        Some(Value::Number(raw)) => raw
            .parse()
            .map_err(|_| format!("ID token has invalid {key}")),
        _ => Err(format!("ID token has no valid {key}")),
    }
}

pub(super) fn validate(
    token: &str,
    client_id: &str,
    nonce: &str,
    cancel: &Cancellation,
) -> Result<Claims, String> {
    if token.len() > 32 * 1024 {
        return Err("ID token exceeds size limit".into());
    }
    let parts: Vec<_> = token.split('.').collect();
    if parts.len() != 3 {
        return Err("ID token is malformed".into());
    }
    let header = json::parse(
        &String::from_utf8(wire::decode64(parts[0])?)
            .map_err(|_| "ID token header is invalid UTF-8")?,
    )?;
    if header.get("alg").and_then(Value::as_str) != Some("RS256")
        || header
            .get("typ")
            .and_then(Value::as_str)
            .is_some_and(|t| t != "JWT")
    {
        return Err("ID token uses an unsupported signature algorithm".into());
    }
    let kid = text(&header, "kid")?;
    let jwks = keys_for(kid, cancel)?;
    validate_with_keys(token, client_id, nonce, &jwks, cancel)
}

#[cfg(test)]
mod tests;

fn keys_for(kid: &str, cancel: &Cancellation) -> Result<Value, String> {
    static CACHE: OnceLock<Mutex<Option<(Instant, Value)>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(None));
    let cached = cache
        .lock()
        .map_err(|_| "OpenAI JWKS cache is unavailable")?
        .clone();
    if let Some((at, document)) = cached
        && at.elapsed() < Duration::from_secs(600)
        && has_key(&document, kid)
    {
        return Ok(document);
    }
    let metadata = wire::discovery(cancel)?;
    let uri = metadata
        .get("jwks_uri")
        .and_then(Value::as_str)
        .ok_or("OpenAI discovery has no JWKS URI")?;
    let document = wire::get(uri, cancel)?;
    *cache
        .lock()
        .map_err(|_| "OpenAI JWKS cache is unavailable")? =
        Some((Instant::now(), document.clone()));
    Ok(document)
}

fn has_key(jwks: &Value, kid: &str) -> bool {
    jwks.get("keys")
        .and_then(Value::as_array)
        .is_some_and(|keys| {
            keys.iter()
                .any(|key| key.get("kid").and_then(Value::as_str) == Some(kid))
        })
}

fn validate_with_keys(
    token: &str,
    client_id: &str,
    nonce: &str,
    jwks: &Value,
    cancel: &Cancellation,
) -> Result<Claims, String> {
    let parts: Vec<_> = token.split('.').collect();
    if parts.len() != 3 {
        return Err("ID token is malformed".into());
    }
    let header = json::parse(
        &String::from_utf8(wire::decode64(parts[0])?)
            .map_err(|_| "ID token header is invalid UTF-8")?,
    )?;
    let kid = text(&header, "kid")?;
    let payload = json::parse(
        &String::from_utf8(wire::decode64(parts[1])?)
            .map_err(|_| "ID token body is invalid UTF-8")?,
    )?;
    let signature = wire::decode64(parts[2])?;
    let keys = jwks
        .get("keys")
        .and_then(Value::as_array)
        .ok_or("OpenAI JWKS has no keys")?;
    let key = keys
        .iter()
        .find(|key| {
            key.get("kid").and_then(Value::as_str) == Some(kid)
                && key.get("kty").and_then(Value::as_str) == Some("RSA")
                && key
                    .get("alg")
                    .and_then(Value::as_str)
                    .is_none_or(|alg| alg == "RS256")
                && key
                    .get("use")
                    .and_then(Value::as_str)
                    .is_none_or(|use_| use_ == "sig")
        })
        .ok_or("ID token signing key is unknown")?;
    let modulus = wire::decode64(text(key, "n")?)?;
    let exponent = wire::decode64(text(key, "e")?)?;
    if modulus.len() < 256
        || modulus.len() > 1024
        || exponent.is_empty()
        || exponent.len() > 8
        || signature.len() != modulus.len()
    {
        return Err("ID token signing key is invalid".into());
    }
    let signed = format!("{}.{}", parts[0], parts[1]);
    if !native::verify_rs256(signed.as_bytes(), &signature, &modulus, &exponent, cancel)? {
        return Err("ID token signature verification failed".into());
    }
    if payload.get("iss").and_then(Value::as_str) != Some(wire::AUTHORITY) {
        return Err("ID token issuer mismatch".into());
    }
    let audience = match payload.get("aud") {
        Some(Value::String(value)) => value == client_id,
        Some(Value::Array(values)) => values.iter().any(|v| v.as_str() == Some(client_id)),
        _ => false,
    };
    if !audience {
        return Err("ID token audience mismatch".into());
    }
    if payload.get("nonce").and_then(Value::as_str) != Some(nonce) {
        return Err("ID token nonce mismatch".into());
    }
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| "System clock is invalid")?
        .as_secs();
    if number(&payload, "exp")? <= now {
        return Err("ID token expired".into());
    }
    let issued = number(&payload, "iat")?;
    if issued > now.saturating_add(5) {
        return Err("ID token issued in the future".into());
    }
    if let Some(Value::Number(nbf)) = payload.get("nbf") {
        let valid_from: u64 = nbf.parse().map_err(|_| "ID token not-before is invalid")?;
        if valid_from > now.saturating_add(5) {
            return Err("ID token is not yet valid".into());
        }
    }
    if matches!(payload.get("aud"), Some(Value::Array(values)) if values.len() > 1)
        && payload.get("azp").and_then(Value::as_str) != Some(client_id)
    {
        return Err("ID token authorized party mismatch".into());
    }
    let subject = text(&payload, "sub")?.to_string();
    let email = payload
        .get("email")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let label = payload
        .get("name")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .unwrap_or(&email)
        .to_string();
    Ok(Claims {
        subject,
        email,
        label: if label.is_empty() {
            "ChatGPT account".into()
        } else {
            label
        },
    })
}
