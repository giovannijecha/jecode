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
/// Accepts integers, integral floats and numeric strings, as servers send all three.
pub(super) fn seconds(value: &Value, key: &str) -> Option<u64> {
    let text = match value.get(key)? {
        Value::Number(text) | Value::String(text) => text.trim(),
        _ => return None,
    };
    text.parse::<u64>().ok().or_else(|| {
        let n = text.parse::<f64>().ok()?;
        (n.is_finite() && n >= 0.0 && n.fract() == 0.0 && n < 1e15).then_some(n as u64)
    })
}
/// Auth-params of the Bearer challenges in `WWW-Authenticate` headers (RFC 9110 section 11.6.1).
pub(super) fn challenge_param(challenge: &[(String, String)], name: &str) -> Option<String> {
    challenge
        .iter()
        .filter(|(header, _)| header.eq_ignore_ascii_case("WWW-Authenticate"))
        .find_map(|(_, value)| {
            auth_params(value)
                .into_iter()
                .find(|(scheme, key, _)| scheme == "bearer" && key.eq_ignore_ascii_case(name))
                .map(|(_, _, value)| value)
        })
}
fn auth_params(header: &str) -> Vec<(String, String, String)> {
    let tchar = |c: char| c.is_ascii_alphanumeric() || "!#$%&'*+-.^_`|~".contains(c);
    let chars: Vec<char> = header.chars().collect();
    let skip = |i: &mut usize, set: &str| {
        while *i < chars.len() && set.contains(chars[*i]) {
            *i += 1;
        }
    };
    let (mut i, mut scheme, mut out) = (0, String::new(), Vec::new());
    while i < chars.len() {
        skip(&mut i, " \t,");
        let start = i;
        while i < chars.len() && tchar(chars[i]) {
            i += 1;
        }
        if start == i {
            i += 1;
            continue;
        }
        let token: String = chars[start..i].iter().collect();
        let mut j = i;
        skip(&mut j, " \t");
        if chars.get(j) != Some(&'=') {
            scheme = token.to_ascii_lowercase();
            continue;
        }
        i = j + 1;
        skip(&mut i, " \t");
        let mut value = String::new();
        if chars.get(i) == Some(&'"') {
            i += 1;
            while i < chars.len() && chars[i] != '"' {
                if chars[i] == '\\' && i + 1 < chars.len() {
                    i += 1;
                }
                value.push(chars[i]);
                i += 1;
            }
            i += 1;
        } else {
            while i < chars.len() && tchar(chars[i]) {
                value.push(chars[i]);
                i += 1;
            }
        }
        out.push((scheme.clone(), token.to_ascii_lowercase(), value));
    }
    out
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
        return Err(failure(&format!(
            "OAuth metadata could not be loaded (HTTP {})",
            response.status
        )));
    }
    let text =
        std::str::from_utf8(&response.body).map_err(|_| failure("OAuth metadata is invalid"))?;
    let value = json::parse(text).map_err(|_| failure("OAuth metadata is invalid"))?;
    if !matches!(value, Value::Object(_)) {
        return Err(failure("OAuth metadata is invalid"));
    }
    Ok(Some(value))
}
/// Use the first well-known candidate whose JSON satisfies the caller's checks.
/// Unusable answers fall through; an error is reported only when none succeeds.
pub(super) fn first_valid_json<T>(
    urls: &[String],
    cancel: &Cancellation,
    mut validate: impl FnMut(Value) -> Result<T, Error>,
) -> Result<Option<T>, Error> {
    let mut failed = None;
    for url in urls {
        match get_json(url, cancel) {
            Ok(Some(value)) => match validate(value) {
                Ok(valid) => return Ok(Some(valid)),
                Err(error) if error.kind == ErrorKind::Cancelled => return Err(error),
                Err(error) => failed = Some(error),
            },
            Ok(None) => {}
            Err(error) if error.kind == ErrorKind::Cancelled => return Err(error),
            Err(error) => failed = Some(error),
        }
    }
    failed.map_or(Ok(None), Err)
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
    let value = (body.len() <= 64 * 1024)
        .then(|| std::str::from_utf8(body).ok())
        .flatten()
        .and_then(|text| json::parse(text).ok());
    if !(200..300).contains(&status) {
        return Err(rejection(status, value.as_ref()));
    }
    match value {
        Some(value @ Value::Object(_)) => Ok(value),
        _ => Err(failure("OAuth response is invalid")),
    }
}
/// Only a rejected grant or client asks for a new sign-in; other failures may pass.
pub(super) fn rejection(status: u16, value: Option<&Value>) -> Error {
    let code = value
        .and_then(|value| value.get("error"))
        .and_then(Value::as_str)
        .filter(|code| {
            !code.is_empty()
                && code.len() <= 64
                && code.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
        });
    let detail = code.map_or_else(|| format!("HTTP {status}"), str::to_owned);
    let mut error = match code {
        // GitHub reports a revoked refresh token as `bad_refresh_token`.
        Some("invalid_grant" | "invalid_client" | "unauthorized_client" | "bad_refresh_token") => {
            failure(&format!(
                "OAuth server rejected the session ({detail}); sign in again"
            ))
        }
        _ if status >= 500
            || status == 429
            || matches!(code, Some("temporarily_unavailable" | "server_error")) =>
        {
            Error::new(
                ErrorKind::Transport,
                format!("OAuth server is unavailable ({detail}); try again later"),
            )
        }
        _ => Error::new(
            ErrorKind::Remote,
            format!("OAuth request was rejected ({detail})"),
        ),
    };
    error.status = Some(status);
    error
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn challenge_params_are_parsed_as_tokens_or_quoted_strings() {
        let header = |value: &str| vec![("WWW-Authenticate".to_owned(), value.to_owned())];
        let quoted = header(
            r#"Bearer xresource_metadata="https://evil.test", resource_metadata="https://a.test/m", scope="a \"b\"""#,
        );
        assert_eq!(
            challenge_param(&quoted, "resource_metadata").as_deref(),
            Some("https://a.test/m")
        );
        assert_eq!(
            challenge_param(&quoted, "scope").as_deref(),
            Some("a \"b\"")
        );
        let bare = header("Basic realm=x, Bearer realm=api,scope=read");
        assert_eq!(challenge_param(&bare, "scope").as_deref(), Some("read"));
        assert_eq!(challenge_param(&bare, "realm").as_deref(), Some("api"));
        let other = header(r#"Basic resource_metadata="https://a.test/m""#);
        assert_eq!(challenge_param(&other, "resource_metadata"), None);
    }

    #[test]
    fn lifetimes_accept_integral_floats_and_numeric_strings() {
        for (raw, expected) in [
            ("120", Some(120)),
            ("120.0", Some(120)),
            ("\"3600\"", Some(3600)),
            ("1.5", None),
            ("-1", None),
            ("\"soon\"", None),
            ("null", None),
        ] {
            let value = json::parse(&format!(r#"{{"expires_in":{raw}}}"#)).unwrap();
            assert_eq!(seconds(&value, "expires_in"), expected, "{raw}");
        }
    }

    #[test]
    fn only_rejected_grants_ask_for_a_new_sign_in() {
        let body = |code: &str| json::parse(&format!(r#"{{"error":"{code}"}}"#)).unwrap();
        let error = parse_response(400, body("invalid_grant").encode().as_bytes()).unwrap_err();
        assert_eq!(error.kind, ErrorKind::Authentication);
        assert!(error.message.contains("invalid_grant"));
        let error = parse_response(400, body("invalid_request").encode().as_bytes()).unwrap_err();
        assert_eq!(error.kind, ErrorKind::Remote);
        assert_eq!(
            parse_response(503, b"<html>busy</html>").unwrap_err().kind,
            ErrorKind::Transport
        );
        let error = parse_response(400, body("temporarily_unavailable").encode().as_bytes());
        assert_eq!(error.unwrap_err().kind, ErrorKind::Transport);
    }
}
