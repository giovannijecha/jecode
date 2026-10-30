use crate::cancel::Cancellation;
use crate::json::{self, Value};
use std::time::Duration;

pub(super) const AUTHORITY: &str = "https://auth.openai.com";
pub(super) const AUTHORIZE: &str = "https://auth.openai.com/api/accounts/authorize";
pub(super) const TOKEN: &str = "https://auth.openai.com/api/accounts/oauth/token";
pub(super) const DISCOVERY: &str = "https://auth.openai.com/.well-known/openid-configuration";
pub(super) const RESOURCE: &str = "https://api.openai.com/v1";
pub(super) const SCOPES: &str =
    "openid profile email offline_access resource.invoke chatgpt.tokens.use.direct";

pub(super) fn base64url(bytes: &[u8]) -> String {
    crate::attachments::base64::encode(bytes)
        .trim_end_matches('=')
        .replace('+', "-")
        .replace('/', "_")
}

pub(super) fn decode64(text: &str) -> Result<Vec<u8>, String> {
    if text.is_empty()
        || text.len() % 4 == 1
        || !text
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        return Err("Invalid base64url value".into());
    }
    let mut standard = text.replace('-', "+").replace('_', "/");
    while !standard.len().is_multiple_of(4) {
        standard.push('=');
    }
    let bytes = crate::attachments::base64::decode(&standard)?;
    if base64url(&bytes) != text {
        return Err("Noncanonical base64url value".into());
    }
    Ok(bytes)
}

pub(super) fn encode_component(text: &str) -> String {
    let mut result = String::new();
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || b"-._~".contains(&byte) {
            result.push(byte as char);
        } else {
            result.push_str(&format!("%{byte:02X}"));
        }
    }
    result
}

fn decode_component(text: &str) -> Result<String, String> {
    let bytes = text.as_bytes();
    let mut output = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'%' if index + 2 < bytes.len() => {
                let hex = std::str::from_utf8(&bytes[index + 1..index + 3])
                    .map_err(|_| "Invalid callback encoding")?;
                output.push(u8::from_str_radix(hex, 16).map_err(|_| "Invalid callback encoding")?);
                index += 3;
            }
            b'%' => return Err("Invalid callback encoding".into()),
            b'+' => {
                output.push(b' ');
                index += 1;
            }
            byte => {
                output.push(byte);
                index += 1;
            }
        }
    }
    String::from_utf8(output).map_err(|_| "Invalid callback UTF-8".into())
}

pub(super) fn query(text: &str) -> Result<std::collections::BTreeMap<String, String>, String> {
    let mut values = std::collections::BTreeMap::new();
    if text.is_empty() {
        return Ok(values);
    }
    for pair in text.split('&') {
        let (key, value) = pair.split_once('=').ok_or("Invalid callback parameter")?;
        let key = decode_component(key)?;
        if values.insert(key, decode_component(value)?).is_some() {
            return Err("Duplicate callback parameter".into());
        }
    }
    Ok(values)
}

pub(super) fn form(fields: &[(&str, &str)]) -> String {
    fields
        .iter()
        .map(|(key, value)| format!("{}={}", encode_component(key), encode_component(value)))
        .collect::<Vec<_>>()
        .join("&")
}

fn request(url: &str, body: Option<&str>, cancel: &Cancellation) -> Result<Value, String> {
    if !url.starts_with("https://auth.openai.com/") {
        return Err("Unexpected OpenAI identity endpoint".into());
    }
    #[cfg(test)]
    let fixture = TEST_ORIGIN
        .lock()
        .map_err(|_| "Token fixture lock failed")?
        .clone();
    #[cfg(test)]
    let endpoint = fixture.as_ref().map_or_else(
        || url.to_string(),
        |origin| url.replacen(AUTHORITY, origin, 1),
    );
    #[cfg(not(test))]
    let endpoint = url.to_string();
    let mut headers = vec![("Accept", "application/json")];
    if body.is_some() {
        headers.push(("Content-Type", "application/x-www-form-urlencoded"));
    }
    let mut decoder = crate::http::Decoder::default();
    let mut response = Vec::new();
    let output = crate::http::run(
        crate::http::Request {
            url: &endpoint,
            method: if body.is_some() { "POST" } else { "GET" },
            headers: &headers,
            body: body.map(str::as_bytes),
            timeout: Some(Duration::from_secs(30)),
            idle: None,
            output_limit: 0,
        },
        cancel,
        &mut |bytes| {
            decoder.feed(bytes, &mut |part| {
                if response.len().saturating_add(part.len()) > 1024 * 1024 {
                    return Err("OpenAI identity response was too large".into());
                }
                response.extend_from_slice(part);
                Ok(())
            })
        },
    )?;
    if output.cancelled || cancel.requested() {
        return Err("Operation cancelled".into());
    }
    if output.timed_out || output.exit_code != Some(0) || output.input_error.is_some() {
        return Err("Could not complete OpenAI identity request".into());
    }
    let body = String::from_utf8(response).map_err(|_| "OpenAI identity response was not UTF-8")?;
    let status = decoder
        .status
        .ok_or("OpenAI identity response had no status")?;
    if !(200..300).contains(&status) {
        let error = json::parse(&body)
            .ok()
            .and_then(|v| match v.get("error") {
                Some(Value::String(code)) => Some(code.clone()),
                Some(Value::Object(_)) => v
                    .get("error")
                    .and_then(|e| e.get("code"))
                    .and_then(Value::as_str)
                    .map(str::to_string),
                _ => None,
            })
            .unwrap_or_else(|| format!("HTTP {status}"));
        // The remote body can contain credentials and is never propagated.
        return Err(format!(
            "OpenAI identity request failed: {}",
            if matches!(
                error.as_str(),
                "invalid_grant"
                    | "invalid_client"
                    | "invalid_refresh_token"
                    | "token_expired"
                    | "refresh_token_expired"
                    | "refresh_token_invalidated"
                    | "refresh_token_reused"
            ) {
                error
            } else {
                format!("HTTP {status}")
            }
        ));
    }
    if body.trim().is_empty() {
        return Ok(Value::Null);
    }
    json::parse(&body).map_err(|_| "OpenAI identity response was invalid JSON".into())
}

pub(super) fn machine_code(error: &str) -> Option<&str> {
    let code = error.strip_prefix("OpenAI identity request failed: ")?;
    if !code.is_empty()
        && code
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
    {
        Some(code)
    } else {
        None
    }
}

#[cfg(test)]
static TEST_ORIGIN: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

#[cfg(test)]
pub(super) fn set_fixture(origin: Option<String>) {
    *TEST_ORIGIN.lock().unwrap() = origin;
}

pub(super) fn get(url: &str, cancel: &Cancellation) -> Result<Value, String> {
    request(url, None, cancel)
}
pub(super) fn post(
    url: &str,
    fields: &[(&str, &str)],
    cancel: &Cancellation,
) -> Result<Value, String> {
    request(url, Some(&form(fields)), cancel)
}

pub(super) fn discovery(cancel: &Cancellation) -> Result<Value, String> {
    let document = get(DISCOVERY, cancel)?;
    if document.get("issuer").and_then(Value::as_str) != Some(AUTHORITY)
        || document
            .get("authorization_endpoint")
            .and_then(Value::as_str)
            != Some(AUTHORIZE)
        || document.get("token_endpoint").and_then(Value::as_str) != Some(TOKEN)
    {
        return Err("Unexpected OpenAI identity discovery document".into());
    }
    Ok(document)
}
