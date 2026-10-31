use super::{
    SignInUpdate,
    storage::{self, Record},
};
use crate::auth::{openai, openai::native};
use crate::cancel::Cancellation;
use crate::json::{self, Value};
use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::time::{Duration, Instant};

const CLIENT_ID: &str = "9d1c250a-e61b-44d9-88ed-5944d1962f5e";
const AUTHORIZE: &str = "https://claude.ai/oauth/authorize";
const TOKEN: &str = "https://platform.claude.com/v1/oauth/token";
const PROFILE: &str = "https://api.anthropic.com/api/oauth/profile";
const SCOPES: &str = "user:profile user:inference";

pub(super) fn sign_in(
    cancel: &Cancellation,
    emit: &mut dyn FnMut(SignInUpdate) -> Result<(), String>,
) -> Result<Record, String> {
    let listener = TcpListener::bind("127.0.0.1:0")
        .map_err(|_| "Could not start Claude sign-in callback listener")?;
    listener
        .set_nonblocking(true)
        .map_err(|_| "Could not configure Claude sign-in callback listener")?;
    let port = listener
        .local_addr()
        .map_err(|_| "Could not read callback address")?
        .port();
    let redirect = format!("http://localhost:{port}/callback");
    let state = base64url(&native::random(32, cancel)?);
    let verifier = base64url(&native::random(32, cancel)?);
    let challenge = base64url(&native::sha256(verifier.as_bytes(), cancel)?);
    let query = form(&[
        ("code", "true"),
        ("client_id", CLIENT_ID),
        ("response_type", "code"),
        ("redirect_uri", &redirect),
        ("scope", SCOPES),
        ("code_challenge", &challenge),
        ("code_challenge_method", "S256"),
        ("state", &state),
    ]);
    let url = format!("{AUTHORIZE}?{query}");
    emit(SignInUpdate::Browser(url.clone()))?;
    let _ = openai::open_browser(&url);
    emit(SignInUpdate::Waiting)?;
    let code = await_callback(&listener, port, &state, cancel)?;
    let body = Value::object([
        ("grant_type", Value::string("authorization_code")),
        ("client_id", Value::string(CLIENT_ID)),
        ("code", Value::string(&code)),
        ("state", Value::string(&state)),
        ("redirect_uri", Value::string(&redirect)),
        ("code_verifier", Value::string(&verifier)),
    ]);
    let value = request(TOKEN, Some(&body), None, cancel)?;
    let mut record = token_record(&value, None)?;
    let (id, label) = profile(&record.access_token, cancel)?;
    record.id = id;
    record.label = label;
    Ok(record)
}

pub(super) fn refresh(record: &Record, cancel: &Cancellation) -> Result<Record, String> {
    if record.refresh_token.is_empty() {
        return Err("Claude account has no refresh token; sign in again".into());
    }
    let body = Value::object([
        ("grant_type", Value::string("refresh_token")),
        ("client_id", Value::string(CLIENT_ID)),
        ("refresh_token", Value::string(&record.refresh_token)),
    ]);
    let value = request(TOKEN, Some(&body), None, cancel)?;
    token_record(&value, Some(record))
}

pub(super) fn invalid_grant(error: &str) -> bool {
    error == "Claude identity request failed: invalid_grant"
        || error == "Claude identity request failed: invalid_refresh_token"
}

fn token_record(value: &Value, previous: Option<&Record>) -> Result<Record, String> {
    let access_token = required(value, "access_token")?.to_owned();
    if access_token.is_empty() {
        return Err("Claude returned an empty access token".into());
    }
    if value
        .get("token_type")
        .and_then(Value::as_str)
        .is_some_and(|kind| !kind.eq_ignore_ascii_case("Bearer"))
    {
        return Err("Claude returned an unsupported token type".into());
    }
    let refresh_token = match value.get("refresh_token") {
        Some(Value::String(token)) if !token.is_empty() => token.clone(),
        Some(_) => return Err("Claude returned an empty refresh token".into()),
        None => previous
            .map(|record| record.refresh_token.clone())
            .ok_or("Claude sign-in returned no refresh token")?,
    };
    let scopes = value
        .get("scope")
        .and_then(Value::as_str)
        .or_else(|| value.get("scopes").and_then(Value::as_str))
        .map(str::to_owned)
        .or_else(|| previous.map(|record| record.scopes.clone()))
        .unwrap_or_else(|| SCOPES.into());
    if !scopes
        .split_whitespace()
        .any(|scope| scope == "user:inference")
    {
        return Err("Claude did not grant inference access".into());
    }
    let expires_in = match value.get("expires_in") {
        Some(Value::Number(number)) => number
            .parse::<u64>()
            .map_err(|_| "Claude returned an invalid token lifetime")?,
        _ => return Err("Claude returned no token lifetime".into()),
    };
    if expires_in == 0 {
        return Err("Claude returned an expired token".into());
    }
    Ok(Record {
        id: previous.map_or(String::new(), |record| record.id.clone()),
        label: previous.map_or(String::new(), |record| record.label.clone()),
        access_token,
        refresh_token,
        expires_at: storage::now()?.saturating_add(expires_in),
        scopes,
    })
}

fn required<'a>(value: &'a Value, key: &str) -> Result<&'a str, String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("Claude response omitted {key}"))
}

fn profile(access_token: &str, cancel: &Cancellation) -> Result<(String, String), String> {
    let value = request(PROFILE, None, Some(access_token), cancel)?;
    let account = value
        .get("account")
        .ok_or("Claude profile omitted account")?;
    let id = account
        .get("uuid")
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty() && id.len() <= 256)
        .ok_or("Claude profile omitted account identity")?;
    let label = account
        .get("email")
        .and_then(Value::as_str)
        .filter(|label| !label.is_empty() && label.len() <= 1024)
        .or_else(|| account.get("display_name").and_then(Value::as_str))
        .unwrap_or("Claude account");
    Ok((id.to_owned(), label.to_owned()))
}

fn request(
    url: &str,
    body: Option<&Value>,
    token: Option<&str>,
    cancel: &Cancellation,
) -> Result<Value, String> {
    if url != TOKEN && url != PROFILE {
        return Err("Unexpected Claude identity endpoint".into());
    }
    let bearer = token.map(|token| format!("Bearer {token}"));
    let mut headers = vec![("Accept", "application/json"), ("User-Agent", "jecode/0.1")];
    if body.is_some() {
        headers.push(("Content-Type", "application/json"));
    }
    if let Some(ref bearer) = bearer {
        headers.push(("Authorization", bearer));
        headers.push(("anthropic-version", "2023-06-01"));
        headers.push(("anthropic-beta", "oauth-2025-04-20"));
    }
    let encoded = body.map(Value::encode);
    let mut decoder = crate::http::Decoder::default();
    let mut bytes = Vec::new();
    let output = crate::http::run(
        crate::http::Request {
            url,
            method: if body.is_some() { "POST" } else { "GET" },
            headers: &headers,
            body: encoded.as_deref().map(str::as_bytes),
            timeout: Some(Duration::from_secs(30)),
            idle: None,
            output_limit: 0,
        },
        cancel,
        &mut |chunk| {
            decoder.feed(chunk, &mut |part| {
                if bytes.len().saturating_add(part.len()) > 1024 * 1024 {
                    return Err("Claude identity response was too large".into());
                }
                bytes.extend_from_slice(part);
                Ok(())
            })
        },
    )?;
    if output.cancelled || cancel.requested() {
        return Err("Operation cancelled".into());
    }
    if output.timed_out || output.exit_code != Some(0) || output.input_error.is_some() {
        return Err("Could not complete Claude identity request".into());
    }
    let status = decoder
        .status
        .ok_or("Claude identity response had no status")?;
    let text = String::from_utf8(bytes).map_err(|_| "Claude identity response was not UTF-8")?;
    if !(200..300).contains(&status) {
        let code = json::parse(&text).ok().and_then(|value| {
            value
                .get("error")
                .and_then(|error| {
                    error
                        .as_str()
                        .or_else(|| error.get("type").and_then(Value::as_str))
                })
                .map(str::to_owned)
        });
        let safe = code
            .as_deref()
            .filter(|code| matches!(*code, "invalid_grant" | "invalid_refresh_token"));
        return Err(format!(
            "Claude identity request failed: {}",
            safe.map(str::to_owned)
                .unwrap_or_else(|| format!("HTTP {status}"))
        ));
    }
    json::parse(&text).map_err(|_| "Claude identity response was invalid JSON".into())
}

fn await_callback(
    listener: &TcpListener,
    port: u16,
    state: &str,
    cancel: &Cancellation,
) -> Result<String, String> {
    let deadline = Instant::now() + Duration::from_secs(300);
    while Instant::now() < deadline {
        if cancel.requested() {
            return Err("Operation cancelled".into());
        }
        match listener.accept() {
            Ok((mut stream, peer)) if peer.ip().is_loopback() => {
                if let Some(code) = handle_callback(&mut stream, port, state, cancel)? {
                    return Ok(code);
                }
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                cancel.wait(Duration::from_millis(50))?;
            }
            Err(_) => return Err("Claude callback listener failed".into()),
        }
    }
    Err("Claude sign-in timed out".into())
}

fn handle_callback(
    stream: &mut TcpStream,
    port: u16,
    state: &str,
    cancel: &Cancellation,
) -> Result<Option<String>, String> {
    stream
        .set_read_timeout(Some(Duration::from_millis(200)))
        .map_err(|_| "Could not configure callback connection")?;
    stream
        .set_write_timeout(Some(Duration::from_secs(2)))
        .map_err(|_| "Could not configure callback connection")?;
    let mut bytes = Vec::with_capacity(1024);
    let deadline = Instant::now() + Duration::from_secs(3);
    while !bytes.windows(4).any(|part| part == b"\r\n\r\n") {
        if cancel.requested() {
            return Err("Operation cancelled".into());
        }
        if Instant::now() >= deadline || bytes.len() >= 8192 {
            respond(stream, "400 Bad Request", "Invalid callback");
            return Ok(None);
        }
        let mut part = [0u8; 1024];
        match stream.read(&mut part) {
            Ok(0) => {
                respond(stream, "400 Bad Request", "Invalid callback");
                return Ok(None);
            }
            Ok(size) => bytes.extend_from_slice(&part[..size]),
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
                ) => {}
            Err(_) => {
                respond(stream, "400 Bad Request", "Invalid callback");
                return Ok(None);
            }
        }
    }
    let Ok(request) = std::str::from_utf8(&bytes) else {
        respond(stream, "400 Bad Request", "Invalid callback encoding");
        return Ok(None);
    };
    let Some((head, _)) = request.split_once("\r\n") else {
        respond(stream, "400 Bad Request", "Invalid callback");
        return Ok(None);
    };
    let Some(target) = head
        .strip_prefix("GET ")
        .and_then(|s| s.strip_suffix(" HTTP/1.1"))
    else {
        respond(stream, "405 Method Not Allowed", "Invalid callback");
        return Ok(None);
    };
    if !request
        .split("\r\n")
        .any(|line| line.eq_ignore_ascii_case(&format!("Host: localhost:{port}")))
    {
        respond(stream, "400 Bad Request", "Invalid callback host");
        return Ok(None);
    }
    match callback(target, state) {
        Ok(code) => {
            respond(
                stream,
                "200 OK",
                "Return to Jecode. This tab can be closed.",
            );
            Ok(Some(code))
        }
        Err(error) if error == "Claude authorization was declined" => {
            respond(
                stream,
                "200 OK",
                "Sign-in was not completed. Return to Jecode.",
            );
            Err(error)
        }
        Err(_) => {
            respond(stream, "400 Bad Request", "Invalid callback");
            Ok(None)
        }
    }
}

fn callback(target: &str, expected: &str) -> Result<String, String> {
    let query = target
        .strip_prefix("/callback?")
        .ok_or("Unknown callback path")?;
    let values = query_map(query)?;
    if values.get("state").map(String::as_str) != Some(expected) {
        return Err("Claude sign-in state mismatch".into());
    }
    if values.contains_key("error") {
        return Err("Claude authorization was declined".into());
    }
    values
        .get("code")
        .filter(|code| !code.is_empty() && code.len() <= 4096)
        .cloned()
        .ok_or("Missing Claude authorization code".into())
}

fn query_map(text: &str) -> Result<BTreeMap<String, String>, String> {
    let mut values = BTreeMap::new();
    for pair in text.split('&') {
        let (key, value) = pair.split_once('=').ok_or("Invalid callback parameter")?;
        let key = decode(key)?;
        if values.insert(key, decode(value)?).is_some() {
            return Err("Duplicate callback parameter".into());
        }
    }
    Ok(values)
}

fn decode(text: &str) -> Result<String, String> {
    let bytes = text.as_bytes();
    let mut result = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'%' if index + 2 < bytes.len() => {
                let hex = std::str::from_utf8(&bytes[index + 1..index + 3])
                    .map_err(|_| "Invalid callback escape")?;
                result.push(u8::from_str_radix(hex, 16).map_err(|_| "Invalid callback escape")?);
                index += 3;
            }
            b'%' => return Err("Invalid callback escape".into()),
            b'+' => {
                result.push(b' ');
                index += 1;
            }
            byte => {
                result.push(byte);
                index += 1;
            }
        }
    }
    String::from_utf8(result).map_err(|_| "Invalid callback UTF-8".into())
}

fn form(fields: &[(&str, &str)]) -> String {
    fields
        .iter()
        .map(|(key, value)| format!("{}={}", encode(key), encode(value)))
        .collect::<Vec<_>>()
        .join("&")
}

fn encode(value: &str) -> String {
    let mut result = String::new();
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || b"-._~".contains(&byte) {
            result.push(byte as char);
        } else {
            result.push_str(&format!("%{byte:02X}"));
        }
    }
    result
}

fn base64url(bytes: &[u8]) -> String {
    crate::attachments::base64::encode(bytes)
        .trim_end_matches('=')
        .replace('+', "-")
        .replace('/', "_")
}

fn respond(stream: &mut TcpStream, status: &str, body: &str) {
    let reply = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\nCache-Control: no-store\r\nReferrer-Policy: no-referrer\r\nContent-Security-Policy: default-src 'none'; frame-ancestors 'none'\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(reply.as_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn callback_checks_state_and_single_code() {
        assert_eq!(
            callback("/callback?code=a%2Bb&state=expected", "expected").unwrap(),
            "a+b"
        );
        assert!(callback("/callback?code=abc&state=wrong", "expected").is_err());
        assert!(callback("/callback?code=a&code=b&state=expected", "expected").is_err());
        assert!(callback("/callback?error=denied&state=expected", "expected").is_err());
    }

    #[test]
    fn token_response_rejects_missing_inference_and_expiry() {
        let base = Value::object([
            ("access_token", Value::string("access")),
            ("refresh_token", Value::string("refresh")),
            ("expires_in", Value::number(3600)),
            ("scope", Value::string("user:profile")),
        ]);
        assert!(token_record(&base, None).is_err());
        let value = Value::object([
            ("access_token", Value::string("access")),
            ("refresh_token", Value::string("refresh")),
            ("expires_in", Value::number(0)),
            ("scope", Value::string(SCOPES)),
        ]);
        assert!(token_record(&value, None).is_err());
    }

    #[test]
    fn refresh_response_replaces_rotated_token_and_keeps_identity() {
        let previous = Record {
            id: "account-uuid".into(),
            label: "person@example.test".into(),
            access_token: "old-access".into(),
            refresh_token: "old-refresh".into(),
            expires_at: 1,
            scopes: SCOPES.into(),
        };
        let response = Value::object([
            ("access_token", Value::string("new-access")),
            ("refresh_token", Value::string("new-refresh")),
            ("expires_in", Value::number(3600)),
        ]);
        let updated = token_record(&response, Some(&previous)).unwrap();
        assert_eq!(updated.id, previous.id);
        assert_eq!(updated.refresh_token, "new-refresh");
        assert_eq!(updated.access_token, "new-access");
        assert!(updated.expires_at > storage::now().unwrap());
    }
}
