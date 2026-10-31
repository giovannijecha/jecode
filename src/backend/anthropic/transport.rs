//! Native HTTP boundary for account metadata and streamed Messages.

use super::Api;
use crate::backend::{Failure, FailureKind, FailureReason};
use crate::json::{self, Value};
use crate::redact::Redactor;
use std::time::Duration;

const DIAGNOSTIC_LIMIT: usize = 64 * 1024;
const VERSION: &str = "2023-06-01";
const ACCOUNT_BETA: &str = "oauth-2025-04-20";
const GENERATION_BETA: &str = "claude-code-20250219,oauth-2025-04-20";
const GENERATION_AGENT: &str = "claude-cli/2.1.314";
const ACCOUNT_AGENT: &str = "jecode/0.1";

pub(super) struct Response {
    pub status: u16,
    pub request_id: Option<String>,
    pub retry_after: Option<Duration>,
}

impl Api {
    pub(super) fn get(&self, path: &str) -> Result<Value, String> {
        let (response, body) = self
            .raw("GET", path, None, &mut |_| Ok(()))
            .map_err(|error| error.message)?;
        if response.status != 200 {
            return Err(format!(
                "Anthropic model catalog returned HTTP {}",
                response.status
            ));
        }
        json::parse(&body).map_err(|error| self.redactor().text(&error))
    }

    pub(super) fn post_stream(
        &self,
        body: &str,
        observer: &mut dyn FnMut(&[u8]) -> Result<(), String>,
    ) -> Result<Response, Failure> {
        self.raw("POST", "/v1/messages?beta=true", Some(body), observer)
            .map(|(response, _)| response)
    }

    fn raw(
        &self,
        method: &str,
        path: &str,
        body: Option<&str>,
        observer: &mut dyn FnMut(&[u8]) -> Result<(), String>,
    ) -> Result<(Response, String), Failure> {
        let token = self
            .session
            .access_token(&self.cancellation)
            .map_err(Failure::from)?;
        let url = format!("{}{path}", self.base_url);
        let authorization = format!("Bearer {token}");
        let beta = if body.is_some() {
            GENERATION_BETA
        } else {
            ACCOUNT_BETA
        };
        let agent = if body.is_some() {
            GENERATION_AGENT
        } else {
            ACCOUNT_AGENT
        };
        let mut headers = vec![
            ("authorization", authorization.as_str()),
            ("anthropic-version", VERSION),
            ("anthropic-beta", beta),
            ("user-agent", agent),
        ];
        if body.is_some() {
            headers.extend([
                ("x-app", "cli"),
                ("content-type", "application/json"),
                ("accept", "text/event-stream"),
            ]);
        } else {
            headers.push(("accept", "application/json"));
        }
        let mut decoder = Headers {
            collect_body: body.is_none(),
            ..Headers::default()
        };
        let mut observer_failed = false;
        let output = crate::http::run(
            crate::http::Request {
                url: &url,
                method,
                headers: &headers,
                body: body.map(str::as_bytes),
                timeout: None,
                idle: Some(Duration::from_secs(120)),
                output_limit: DIAGNOSTIC_LIMIT,
            },
            &self.cancellation,
            &mut |bytes| {
                let result = decoder.feed(bytes, observer);
                if result.is_err() {
                    observer_failed = true;
                }
                result
            },
        )
        .map_err(|error| {
            let message = self.redactor().text(&error);
            let mut failure = if observer_failed {
                Failure::from(message)
            } else {
                Failure::temporary(message)
            };
            if let Some(status) = decoder.status {
                failure.message.push_str(&format!("; HTTP {status}"));
            }
            failure.retry_after = decoder.retry_after;
            failure
        })?;
        if output.cancelled {
            return Err(Failure::from("Operation cancelled"));
        }
        if output.timed_out {
            return Err(Failure::temporary("Anthropic connection became inactive"));
        }
        if output.exit_code != Some(0) {
            let (_, stderr) = self
                .redactor()
                .byte_prefix(&output.stderr, output.stderr_bytes == output.stderr.len());
            let message = format!(
                "Could not reach Anthropic via native HTTP (exit {}): {}",
                output
                    .exit_code
                    .map_or_else(|| "unknown".into(), |code| code.to_string()),
                String::from_utf8_lossy(&stderr).trim()
            );
            return Err(Failure::temporary(message));
        }
        if let Some(error) = output.input_error {
            return Err(Failure::from(format!(
                "Could not send Anthropic HTTP request: {error}"
            )));
        }
        let status = decoder
            .status
            .ok_or_else(|| Failure::temporary("Anthropic returned no HTTP status"))?;
        let text = if (200..300).contains(&status) {
            String::from_utf8(decoder.body)
                .map_err(|_| Failure::from("Anthropic returned invalid UTF-8"))?
        } else {
            String::from_utf8_lossy(&decoder.body).into_owned()
        };
        if !(200..300).contains(&status) {
            let parsed = json::parse(&text).ok();
            let mut error = failure(parsed.as_ref(), Some(status));
            error.retry_after = decoder.retry_after;
            if parsed.is_none() {
                error.diagnostic = Some(text);
            }
            if let Some(id) = &decoder.request_id {
                error.message.push_str(&format!("; request id: {id}"));
            }
            sanitize(&mut error, &self.redactor());
            return Err(error);
        }
        Ok((
            Response {
                status,
                request_id: decoder.request_id,
                retry_after: decoder.retry_after,
            },
            text,
        ))
    }
}

#[derive(Default)]
struct Headers {
    pending: Vec<u8>,
    body: Vec<u8>,
    status: Option<u16>,
    request_id: Option<String>,
    retry_after: Option<Duration>,
    ready: bool,
    collect_body: bool,
}

impl Headers {
    fn feed(
        &mut self,
        bytes: &[u8],
        observer: &mut dyn FnMut(&[u8]) -> Result<(), String>,
    ) -> Result<(), String> {
        if self.ready {
            return self.body(bytes, observer);
        }
        self.pending.extend_from_slice(bytes);
        loop {
            let Some(end) = self
                .pending
                .windows(4)
                .position(|bytes| bytes == b"\r\n\r\n")
            else {
                if self.pending.len() > 64 * 1024 {
                    return Err("Anthropic HTTP headers are too large".into());
                }
                return Ok(());
            };
            let header = self.pending.drain(..end + 4).collect::<Vec<_>>();
            let header = std::str::from_utf8(&header)
                .map_err(|_| "Anthropic returned invalid HTTP headers")?;
            let status = header
                .lines()
                .next()
                .and_then(|line| line.split_whitespace().nth(1))
                .and_then(|value| value.parse::<u16>().ok())
                .ok_or("Anthropic returned invalid HTTP status")?;
            self.status = Some(status);
            self.request_id = header.lines().find_map(|line| {
                let (name, value) = line.split_once(':')?;
                (name.eq_ignore_ascii_case("request-id")
                    && value.trim().bytes().all(|byte| byte.is_ascii_graphic()))
                .then(|| value.trim().to_owned())
            });
            self.retry_after = header.lines().find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("retry-after")
                    .then(|| crate::backend::retry::retry_delay(value.trim()))
                    .flatten()
            });
            if (100..200).contains(&status) {
                continue;
            }
            self.ready = true;
            let rest = std::mem::take(&mut self.pending);
            return self.body(&rest, observer);
        }
    }

    fn body(
        &mut self,
        bytes: &[u8],
        observer: &mut dyn FnMut(&[u8]) -> Result<(), String>,
    ) -> Result<(), String> {
        if self
            .status
            .is_some_and(|status| (200..300).contains(&status))
        {
            if self.collect_body {
                self.body.extend_from_slice(bytes);
            }
            observer(bytes)
        } else {
            let keep = bytes
                .len()
                .min(DIAGNOSTIC_LIMIT.saturating_sub(self.body.len()));
            self.body.extend_from_slice(&bytes[..keep]);
            Ok(())
        }
    }
}

pub(super) fn failure(value: Option<&Value>, status: Option<u16>) -> Failure {
    let error = value
        .and_then(|value| value.get("error"))
        .or(value)
        .unwrap_or(&Value::Null);
    let kind = error.get("type").and_then(Value::as_str).unwrap_or("");
    let detail = error
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or("Anthropic request failed");
    let context = status == Some(413)
        || matches!(
            kind,
            "model_context_window_exceeded" | "context_length_exceeded"
        )
        || (kind == "invalid_request_error"
            && ["prompt is too long", "context window", "context length"]
                .iter()
                .any(|marker| detail.to_ascii_lowercase().contains(marker)));
    let failure_kind = match (status, kind) {
        _ if context => FailureKind::Context,
        (Some(408 | 425 | 429 | 500..=599), _)
        | (_, "rate_limit_error" | "overloaded_error" | "api_error") => FailureKind::Temporary,
        _ => FailureKind::Terminal,
    };
    Failure {
        kind: failure_kind,
        reason: match (status, kind) {
            (Some(429), _) | (_, "rate_limit_error") => Some(FailureReason::RateLimited),
            (Some(401 | 403), _) | (_, "authentication_error" | "permission_error") => {
                Some(FailureReason::RejectedKey)
            }
            _ => None,
        },
        message: format!(
            "Anthropic {kind}: {detail}{}",
            status.map_or_else(String::new, |status| format!(" (HTTP {status})"))
        ),
        diagnostic: None,
        retry_after: None,
        charges: None,
    }
}

pub(super) fn sanitize(failure: &mut Failure, redactor: &Redactor) {
    failure.message = clip(&failure.message, redactor);
    failure.diagnostic = failure
        .diagnostic
        .as_deref()
        .map(|value| clip(value, redactor));
}

fn clip(value: &str, redactor: &Redactor) -> String {
    let mut value = redactor.text(value);
    if value.len() > 8192 {
        value.truncate(value.floor_char_boundary(8192));
        value.push_str(" [diagnostic truncated]");
    }
    value
}
