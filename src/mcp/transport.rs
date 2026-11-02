//! Bounded HTTP transport for MCP and its authorization endpoints.

use crate::cancel::Cancellation;
use crate::mcp::types::{Error, ErrorKind};
use std::time::Duration;

const BODY_LIMIT: usize = 8 * 1024 * 1024;

pub struct Response {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

pub fn validate_endpoint(endpoint: &str) -> Result<(), Error> {
    let rest = endpoint
        .strip_prefix("https://")
        .or_else(|| endpoint.strip_prefix("http://"))
        .ok_or_else(|| {
            Error::new(
                ErrorKind::Configuration,
                "MCP endpoint must use HTTP or HTTPS",
            )
        })?;
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    let valid_port = |port: &str| port.parse::<u16>().is_ok_and(|number| number > 0);
    let valid_host = if authority.starts_with('[') {
        authority.split_once(']').is_some_and(|(host, suffix)| {
            host.len() > 1
                && (suffix.is_empty() || suffix.strip_prefix(':').is_some_and(valid_port))
        })
    } else {
        let (host, port) = authority.split_once(':').unwrap_or((authority, ""));
        !host.is_empty()
            && !host.contains(':')
            && (port.is_empty() || valid_port(port))
            && (!authority.contains(':') || !port.is_empty())
    };
    if !valid_host
        || authority.contains('@')
        || endpoint.contains('#')
        || endpoint.bytes().any(|b| b <= 32 || b == 127)
    {
        return Err(Error::new(
            ErrorKind::Configuration,
            "Invalid MCP endpoint URL",
        ));
    }
    Ok(())
}

pub fn request(
    endpoint: &str,
    method: &str,
    headers: &[(String, String)],
    body: Option<&[u8]>,
    timeout_secs: u64,
    cancel: &Cancellation,
) -> Result<Response, Error> {
    request_inner(endpoint, method, headers, body, timeout_secs, cancel, None)
}

pub(crate) fn request_rpc(
    endpoint: &str,
    method: &str,
    headers: &[(String, String)],
    body: Option<&[u8]>,
    timeout_secs: u64,
    cancel: &Cancellation,
    id: u64,
) -> Result<Response, Error> {
    request_inner(
        endpoint,
        method,
        headers,
        body,
        timeout_secs,
        cancel,
        Some(id),
    )
}

fn request_inner(
    endpoint: &str,
    method: &str,
    headers: &[(String, String)],
    body: Option<&[u8]>,
    timeout_secs: u64,
    cancel: &Cancellation,
    expected_id: Option<u64>,
) -> Result<Response, Error> {
    validate_endpoint(endpoint)?;
    if cancel.requested() {
        return Err(Error::new(ErrorKind::Cancelled, "MCP request cancelled"));
    }
    if !matches!(method, "GET" | "POST" | "DELETE") {
        return Err(Error::new(
            ErrorKind::Configuration,
            "Invalid MCP HTTP method",
        ));
    }
    if headers.iter().any(|(name, value)| {
        name.is_empty()
            || !name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&byte))
            || value
                .bytes()
                .any(|byte| byte == b'\r' || byte == b'\n' || byte == 0)
    }) {
        return Err(Error::new(
            ErrorKind::Configuration,
            "Invalid MCP request header",
        ));
    }
    if body.is_some_and(|bytes| bytes.len() > BODY_LIMIT) {
        return Err(Error::new(
            ErrorKind::Configuration,
            "MCP request is too large",
        ));
    }
    let pairs: Vec<_> = headers
        .iter()
        .map(|(name, value)| (name.as_str(), value.as_str()))
        .collect();
    let mut decoder = crate::http::Decoder::default();
    let mut received = Vec::new();
    let mut sse = super::wire::SseScanner::default();
    let mut stream_complete = false;
    const COMPLETE: &str = "MCP_RESPONSE_RECEIVED";
    let timeout = Duration::from_secs(timeout_secs.clamp(1, 600));
    let output = crate::http::run(
        crate::http::Request {
            url: endpoint,
            method,
            headers: &pairs,
            body,
            timeout: Some(timeout),
            idle: None,
            output_limit: 0,
        },
        cancel,
        &mut |bytes| {
            decoder.feed(bytes, &mut |part| {
                if received.len().saturating_add(part.len()) > BODY_LIMIT {
                    return Err("MCP HTTP response is too large".into());
                }
                received.extend_from_slice(part);
                Ok(())
            })?;
            let is_sse = decoder.header("Content-Type").is_some_and(|value| {
                value
                    .split(';')
                    .next()
                    .is_some_and(|media| media.trim().eq_ignore_ascii_case("text/event-stream"))
            });
            if is_sse
                && decoder
                    .status
                    .is_some_and(|status| (200..300).contains(&status))
                && let Some(id) = expected_id
                && let Some(end) = sse.response_end(&received, id)
            {
                received.truncate(end);
                stream_complete = true;
                return Err(COMPLETE.into());
            }
            Ok(())
        },
    );
    // The observer's stop wins over any process cleanup detail appended to it,
    // so a received response stays complete whatever followed it.
    let early_complete = stream_complete;
    if !early_complete && output.is_err() {
        return Err(Error::new(ErrorKind::Transport, "MCP HTTP request failed"));
    }
    if !early_complete
        && (output.as_ref().is_ok_and(|output| output.cancelled) || cancel.requested())
    {
        return Err(Error::new(ErrorKind::Cancelled, "MCP request cancelled"));
    }
    if !early_complete && output.as_ref().is_ok_and(|output| output.timed_out) {
        return Err(Error::new(ErrorKind::Transport, "MCP request timed out"));
    }
    if !early_complete
        && output
            .as_ref()
            .is_ok_and(|output| output.exit_code != Some(0) || output.input_error.is_some())
    {
        return Err(Error::new(ErrorKind::Transport, "MCP HTTP request failed"));
    }
    let status = decoder
        .status
        .ok_or_else(|| Error::new(ErrorKind::Protocol, "MCP response had no HTTP status"))?;
    if status == 401 {
        let mut error = Error::new(ErrorKind::Authentication, "MCP authentication required");
        error.status = Some(status);
        error.headers = decoder
            .headers
            .into_iter()
            .filter(|(name, _)| name.eq_ignore_ascii_case("WWW-Authenticate"))
            .collect();
        return Err(error);
    }
    Ok(Response {
        status,
        headers: decoder.headers,
        body: received,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::server::{Server, accept};
    use std::io::Write;

    #[test]
    fn rejects_ambiguous_or_credentialed_endpoints() {
        for endpoint in [
            "ftp://example.test/mcp",
            "http://user:secret@example.test/mcp",
            "https://example.test/mcp#fragment",
            "http://example.test:abc/mcp",
            "http://example.test/a\r\nb",
            "http://",
        ] {
            assert!(validate_endpoint(endpoint).is_err(), "{endpoint}");
        }
        assert!(validate_endpoint("http://127.0.0.1:8080/mcp").is_ok());
        assert!(validate_endpoint("https://example.test/mcp?q=one").is_ok());
    }

    #[test]
    fn authentication_challenge_is_returned_without_response_body() {
        let server = Server::start(|listener, stop| {
            let mut stream = accept(&listener, &stop).unwrap();
            write!(stream, "HTTP/1.1 401 Unauthorized\r\nWWW-Authenticate: Bearer resource_metadata=\"https://example.test/auth\"\r\nContent-Length: 6\r\n\r\nsecret").unwrap();
        });
        let error = request(
            &server.endpoint,
            "GET",
            &[],
            None,
            5,
            &Cancellation::default(),
        )
        .err()
        .unwrap();
        assert_eq!(error.kind, ErrorKind::Authentication);
        assert_eq!(error.status, Some(401));
        assert_eq!(error.headers.len(), 1);
        assert!(!error.message.contains("secret"));
        server.finish();
    }
}
