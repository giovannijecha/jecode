//! Experimental device login protocol. All responses require authenticated HTTPS.
//! No credential discovery, persistence, browser launch or network I/O is performed.
mod device;
mod refresh;
mod saved;
mod tokens;
use crate::{
    http,
    json::{self, Value},
};
pub use device::{DeviceLogin, PollOutcome};
use std::fmt;
pub use tokens::Tokens;
pub(crate) use tokens::new_generation;

const CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
pub const AUTH_HOST: &str = "auth.openai.com";
pub const VERIFICATION_URL: &str = "https://auth.openai.com/codex/device";
const REDIRECT_URI: &str = "https://auth.openai.com/deviceauth/callback";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    Invalid,
    State,
    Expired,
    Denied,
    Remote,
    Closed,
}
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Invalid => "invalid account authentication response",
            Self::State => "account authentication operation is out of order",
            Self::Expired => "account authentication attempt expired",
            Self::Denied => "account authentication was denied",
            Self::Remote => "account authentication request failed",
            Self::Closed => "account authentication attempt is closed",
        })
    }
}
impl std::error::Error for Error {}

pub fn start_request() -> Result<Vec<u8>, Error> {
    json_request(
        "/api/accounts/deviceauth/usercode",
        json::object([("client_id", text(CLIENT_ID))]),
    )
}
fn json_request(path: &str, value: Value) -> Result<Vec<u8>, Error> {
    let body = json::encode(&value, 32768).map_err(|_| Error::Invalid)?;
    http::post_json(
        AUTH_HOST,
        path,
        &[("Accept", "application/json")],
        &body,
        65536,
    )
    .map_err(|_| Error::Invalid)
}
fn parse(body: &str) -> Result<Value, Error> {
    let value = json::parse(
        body,
        json::Limits {
            bytes: 32768,
            nodes: 1024,
            depth: 16,
        },
    )
    .map_err(|_| Error::Invalid)?;
    if !matches!(value, Value::Object(_)) {
        return Err(Error::Invalid);
    }
    Ok(value)
}
fn required<'a>(value: &'a Value, key: &str, limit: usize) -> Result<&'a str, Error> {
    value
        .get(key)
        .and_then(Value::text)
        .filter(|text| {
            !text.is_empty() && text.len() <= limit && text.bytes().all(|b| (33..=126).contains(&b))
        })
        .ok_or(Error::Invalid)
}
fn text(value: &str) -> Value {
    Value::String(value.to_owned())
}
fn form(pairs: &[(&str, &str)]) -> String {
    let mut body = String::new();
    for (index, (key, value)) in pairs.iter().enumerate() {
        if index != 0 {
            body.push('&');
        }
        body.push_str(key);
        body.push('=');
        for byte in value.bytes() {
            if byte.is_ascii_alphanumeric() || b"-._~".contains(&byte) {
                body.push(byte as char);
            } else {
                use fmt::Write;
                let _ = write!(body, "%{byte:02X}");
            }
        }
    }
    body
}
