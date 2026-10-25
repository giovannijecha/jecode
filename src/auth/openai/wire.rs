use crate::cancel::Cancellation;
use crate::json::{self, Value};
use std::process::Command;
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
    let mut command = Command::new(crate::curl::executable());
    command.args([
        "--disable",
        "--silent",
        "--show-error",
        "--proto",
        if endpoint.starts_with("https://") {
            "=https"
        } else {
            "=http"
        },
        "--max-time",
        "30",
        "--connect-timeout",
        "10",
        "--request",
        if body.is_some() { "POST" } else { "GET" },
        "--header",
        "Accept: application/json",
        "--write-out",
        "\n%{http_code}",
        "--url",
        &endpoint,
    ]);
    #[cfg(test)]
    if fixture.is_some() {
        command.args(["--noproxy", "*"]);
    }
    if body.is_some() {
        command.args([
            "--header",
            "Content-Type: application/x-www-form-urlencoded",
            "--data-binary",
            "@-",
        ]);
    }
    crate::auth::isolate_environment(&mut command);
    let output = crate::curl::run(
        &mut command,
        body.map(|s| s.as_bytes().to_vec()),
        Some(Duration::from_secs(35)),
        1024 * 1024,
        cancel,
        None,
        &mut |_| Ok(()),
    )?;
    if output.cancelled || cancel.requested() {
        return Err("Operation cancelled".into());
    }
    if output.timed_out
        || output.exit_code != Some(0)
        || output.stdout_truncated
        || output.input_error.is_some()
    {
        return Err("Could not complete OpenAI identity request".into());
    }
    let text =
        String::from_utf8(output.stdout).map_err(|_| "OpenAI identity response was not UTF-8")?;
    let (body, code) = text
        .rsplit_once('\n')
        .ok_or("OpenAI identity response had no status")?;
    let status: u16 = code
        .trim()
        .parse()
        .map_err(|_| "OpenAI identity response had an invalid status")?;
    if !(200..300).contains(&status) {
        let error = json::parse(body)
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
    json::parse(body).map_err(|_| "OpenAI identity response was invalid JSON".into())
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
