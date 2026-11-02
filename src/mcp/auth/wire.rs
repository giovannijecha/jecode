use super::super::{
    config, transport,
    types::{Error, ErrorKind},
};
use crate::{
    cancel::Cancellation,
    json::{self, Value},
};

pub(super) fn failure(message: &str) -> Error {
    Error::new(ErrorKind::Authentication, message)
}
pub(super) fn required<'a>(value: &'a Value, key: &str) -> Result<&'a str, Error> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| failure("OAuth response is missing a required field"))
}
pub(super) fn seconds(value: &Value, key: &str) -> Option<u64> {
    value.get(key).and_then(Value::as_usize).map(|n| n as u64)
}
pub(super) fn origin(url: &str) -> Result<String, Error> {
    config::validate_url(url)?;
    let (scheme, rest) = url
        .split_once("://")
        .ok_or_else(|| failure("OAuth URL has no scheme"))?;
    Ok(format!(
        "{scheme}://{}",
        rest.split(['/', '?']).next().unwrap_or("")
    ))
}
pub(super) fn encode(value: &str) -> String {
    let mut out = String::new();
    for b in value.bytes() {
        if b.is_ascii_alphanumeric() || b"-._~".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}
pub(super) fn form(fields: &[(&str, &str)]) -> String {
    fields
        .iter()
        .map(|(k, v)| format!("{}={}", encode(k), encode(v)))
        .collect::<Vec<_>>()
        .join("&")
}
pub(super) fn query(text: &str) -> Result<std::collections::BTreeMap<String, String>, Error> {
    let mut map = std::collections::BTreeMap::new();
    for part in text.split('&') {
        let (k, v) = part
            .split_once('=')
            .ok_or_else(|| failure("OAuth callback query is invalid"))?;
        let decode = |s: &str| -> Result<String, Error> {
            let mut out = Vec::new();
            let bytes = s.as_bytes();
            let mut i = 0;
            while i < bytes.len() {
                if bytes[i] == b'%' {
                    if i + 2 >= bytes.len() {
                        return Err(failure("OAuth callback encoding is invalid"));
                    }
                    let h = std::str::from_utf8(&bytes[i + 1..i + 3])
                        .map_err(|_| failure("OAuth callback encoding is invalid"))?;
                    out.push(
                        u8::from_str_radix(h, 16)
                            .map_err(|_| failure("OAuth callback encoding is invalid"))?,
                    );
                    i += 3;
                } else {
                    out.push(if bytes[i] == b'+' { b' ' } else { bytes[i] });
                    i += 1;
                }
            }
            String::from_utf8(out).map_err(|_| failure("OAuth callback encoding is invalid"))
        };
        if map.insert(decode(k)?, decode(v)?).is_some() {
            return Err(failure("OAuth callback has duplicate fields"));
        }
    }
    Ok(map)
}
pub(super) fn base64url(bytes: &[u8]) -> String {
    crate::attachments::base64::encode(bytes)
        .trim_end_matches('=')
        .replace('+', "-")
        .replace('/', "_")
}
pub(super) fn get_json(url: &str, cancel: &Cancellation) -> Result<Option<Value>, Error> {
    config::validate_url(url)?;
    let response = transport::request(
        url,
        "GET",
        &[("Accept".into(), "application/json".into())],
        None,
        15,
        cancel,
    )?;
    if response.status == 404 {
        return Ok(None);
    }
    if response.status != 200 || response.body.len() > 64 * 1024 {
        return Err(failure("OAuth metadata could not be loaded"));
    }
    let text =
        std::str::from_utf8(&response.body).map_err(|_| failure("OAuth metadata is invalid"))?;
    let value = json::parse(text).map_err(|_| failure("OAuth metadata is invalid"))?;
    if !matches!(value, Value::Object(_)) {
        return Err(failure("OAuth metadata is invalid"));
    }
    Ok(Some(value))
}
pub(super) fn post_json(url: &str, body: &Value, cancel: &Cancellation) -> Result<Value, Error> {
    config::validate_url(url)?;
    let data = body.encode();
    let response = transport::request(
        url,
        "POST",
        &[
            ("Accept".into(), "application/json".into()),
            ("Content-Type".into(), "application/json".into()),
        ],
        Some(data.as_bytes()),
        15,
        cancel,
    )?;
    parse_response(response.status, &response.body)
}
pub(super) fn post_form(
    url: &str,
    fields: &[(&str, &str)],
    cancel: &Cancellation,
) -> Result<Value, Error> {
    config::validate_url(url)?;
    let data = form(fields);
    let response = transport::request(
        url,
        "POST",
        &[
            ("Accept".into(), "application/json".into()),
            (
                "Content-Type".into(),
                "application/x-www-form-urlencoded".into(),
            ),
        ],
        Some(data.as_bytes()),
        15,
        cancel,
    )?;
    parse_response(response.status, &response.body)
}
fn parse_response(status: u16, body: &[u8]) -> Result<Value, Error> {
    if body.len() > 64 * 1024 {
        return Err(failure("OAuth response is too large"));
    }
    let text = std::str::from_utf8(body).map_err(|_| failure("OAuth response is invalid"))?;
    let value = json::parse(text).map_err(|_| failure("OAuth response is invalid"))?;
    if !(200..300).contains(&status) {
        return Err(failure("OAuth request was rejected"));
    }
    if !matches!(value, Value::Object(_)) {
        return Err(failure("OAuth response is invalid"));
    }
    Ok(value)
}
