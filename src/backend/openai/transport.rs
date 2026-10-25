//! HTTPS boundary: system curl 8.3+, with tokens only in the child environment.

use super::Api;
use crate::backend::{Failure, FailureKind, FailureReason};
use crate::json::{self, Value};
use crate::redact::Redactor;
use std::process::Command;
use std::time::Duration;

pub(super) struct Response {
    pub body: String,
    pub status: u16,
    pub request_id: Option<String>,
    pub retry_after: Option<Duration>,
}

impl Api {
    pub(super) fn get(&self, path: &str) -> Result<Value, String> {
        let response = self
            .raw("GET", path, None, None, &mut |_| Ok(()))
            .map_err(|error| error.message)?;
        json::parse(&response.body).map_err(|error| self.redactor().text(&error))
    }

    pub(super) fn post_stream(
        &self,
        path: &str,
        body: &str,
        session: Option<&str>,
        observer: &mut dyn FnMut(&[u8]) -> Result<(), String>,
    ) -> Result<Response, Failure> {
        self.raw("POST", path, Some(body), session, observer)
    }

    fn raw(
        &self,
        method: &str,
        path: &str,
        body: Option<&str>,
        session: Option<&str>,
        observer: &mut dyn FnMut(&[u8]) -> Result<(), String>,
    ) -> Result<Response, Failure> {
        let token = self
            .session
            .access_token(&self.cancellation)
            .map_err(Failure::from)?;
        let url = format!("{}{path}", self.base_url);
        let mut command = Command::new(crate::curl::executable());
        command.args([
            "--disable",
            "--silent",
            "--show-error",
            "--no-buffer",
            "--variable",
            "%JECODE_OPENAI_ACCESS_TOKEN",
            "--expand-header",
            "Authorization: Bearer {{JECODE_OPENAI_ACCESS_TOKEN}}",
            "--request",
            method,
            "--connect-timeout",
            "10",
            "--suppress-connect-headers",
            "--dump-header",
            "-",
            "--url",
            &url,
        ]);
        crate::auth::isolate_environment(&mut command);
        command.env("JECODE_OPENAI_ACCESS_TOKEN", &token);
        if body.is_some() {
            command.args([
                "--header",
                "Content-Type: application/json",
                "--header",
                "Accept: text/event-stream",
                "--header",
                "Expect:",
                "--data-binary",
                "@-",
            ]);
        }
        // The ChatGPT route derives prompt cache affinity from this header.
        if let Some(session) = session {
            command.args(["--header", &format!("session-id: {session}")]);
        }
        if self.base_url.starts_with("https://") {
            command.args(["--proto", "=https"]);
        } else {
            command.args(["--noproxy", "*"]);
        }
        let mut decoder = Headers::default();
        let mut observer_failed = false;
        let output = crate::curl::run(
            &mut command,
            body.map(|body| body.as_bytes().to_vec()),
            None,
            usize::MAX,
            &self.cancellation,
            Some(Duration::from_secs(120)),
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
            if let Some(id) = &decoder.request_id {
                failure.message.push_str(&format!("; request id: {id}"));
            }
            failure.retry_after = decoder.retry_after;
            failure
        })?;
        if output.cancelled {
            return Err(Failure::from("Operation cancelled"));
        }
        if output.timed_out {
            return Err(Failure::temporary("OpenAI connection became inactive"));
        }
        if output.exit_code != Some(0) {
            let stderr = self
                .redactor()
                .text(&String::from_utf8_lossy(&output.stderr));
            let message = format!(
                "Could not reach OpenAI via curl (exit {}): {}",
                output
                    .exit_code
                    .map_or_else(|| "unknown".into(), |code| code.to_string()),
                stderr.trim()
            );
            return Err(
                if matches!(
                    output.exit_code,
                    Some(5 | 6 | 7 | 18 | 28 | 35 | 52 | 55 | 56 | 92)
                ) {
                    Failure::temporary(message)
                } else {
                    Failure::from(message)
                },
            );
        }
        if let Some(error) = output.input_error {
            return Err(Failure::from(format!(
                "Could not send OpenAI request to curl: {error}"
            )));
        }
        let status = decoder
            .status
            .ok_or_else(|| Failure::temporary("OpenAI returned no HTTP status"))?;
        let body = String::from_utf8(decoder.body)
            .map_err(|_| Failure::from("OpenAI returned invalid UTF-8"))?;
        if !(200..300).contains(&status) {
            let parsed = json::parse(&body).ok();
            let mut error = failure(parsed.as_ref(), Some(status));
            error.retry_after = decoder.retry_after;
            if parsed.is_none() {
                error.diagnostic = Some(body);
            }
            if let Some(id) = decoder.request_id {
                error.message.push_str(&format!("; request id: {id}"));
            }
            sanitize(&mut error, &self.redactor());
            return Err(error);
        }
        Ok(Response {
            body,
            status,
            request_id: decoder.request_id,
            retry_after: decoder.retry_after,
        })
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
}

impl Headers {
    fn feed(
        &mut self,
        bytes: &[u8],
        observer: &mut dyn FnMut(&[u8]) -> Result<(), String>,
    ) -> Result<(), String> {
        if self.ready {
            self.body.extend_from_slice(bytes);
            if self
                .status
                .is_some_and(|status| (200..300).contains(&status))
            {
                observer(bytes)?;
            }
            return Ok(());
        }
        self.pending.extend_from_slice(bytes);
        loop {
            let Some(end) = self
                .pending
                .windows(4)
                .position(|bytes| bytes == b"\r\n\r\n")
            else {
                if self.pending.len() > 64 * 1024 {
                    return Err("OpenAI HTTP headers are too large".into());
                }
                return Ok(());
            };
            let header = self.pending.drain(..end + 4).collect::<Vec<_>>();
            let header =
                std::str::from_utf8(&header).map_err(|_| "OpenAI returned invalid HTTP headers")?;
            let status = header
                .lines()
                .next()
                .and_then(|line| line.split_whitespace().nth(1))
                .and_then(|status| status.parse::<u16>().ok())
                .ok_or("OpenAI returned invalid HTTP status")?;
            self.status = Some(status);
            self.retry_after = header.lines().find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("retry-after")
                    .then(|| crate::backend::retry::retry_delay(value.trim()))
                    .flatten()
            });
            self.request_id = header.lines().find_map(|line| {
                let (name, value) = line.split_once(':')?;
                (name.eq_ignore_ascii_case("x-request-id")
                    && value.trim().bytes().all(|byte| byte.is_ascii_graphic()))
                .then(|| value.trim().to_owned())
            });
            if (100..200).contains(&status) {
                continue;
            }
            self.ready = true;
            let rest = std::mem::take(&mut self.pending);
            self.body.extend_from_slice(&rest);
            if (200..300).contains(&status) {
                observer(&rest)?;
            }
            return Ok(());
        }
    }
}

pub(super) fn failure(value: Option<&Value>, status: Option<u16>) -> Failure {
    let error = value
        .and_then(|value| value.get("error"))
        .unwrap_or(&Value::Null);
    let code = error.get("code").and_then(Value::as_str).unwrap_or("");
    let param = error.get("param").and_then(Value::as_str).unwrap_or("");
    let detail = error
        .get("message")
        .and_then(Value::as_str)
        .or_else(|| {
            value
                .and_then(|value| value.get("detail"))
                .and_then(Value::as_str)
        })
        .or_else(|| {
            value
                .and_then(|value| value.get("message"))
                .and_then(Value::as_str)
        })
        .unwrap_or("Request failed");
    let mut message = format!(
        "OpenAI {}: {detail}",
        status.map_or_else(
            || "response failed".into(),
            |status| format!("HTTP {status}")
        )
    );
    if !code.is_empty() {
        message.push_str(&format!("; code: {code}"));
    }
    if !param.is_empty() {
        message.push_str(&format!("; param: {param}"));
    }
    if code == "subscription_sharing_usage_limit_exceeded" {
        message.push_str(". Manage usage at https://chatgpt.com/settings/usage");
    }
    let temporary = matches!(status, Some(408 | 425 | 500..=599))
        || (status == Some(429) && code != "subscription_sharing_usage_limit_exceeded")
        || matches!(
            code,
            "rate_limit_exceeded"
                | "slow_down"
                | "server_is_overloaded"
                | "subscription_sharing_usage_unavailable"
                | "subscription_sharing_user_unavailable"
        );
    Failure {
        kind: if temporary {
            FailureKind::Temporary
        } else if code == "context_length_exceeded" {
            FailureKind::Context
        } else if code == "max_output_tokens" {
            FailureKind::Length
        } else {
            FailureKind::Terminal
        },
        reason: if matches!(code, "rate_limit_exceeded" | "slow_down")
            || (status == Some(429) && code != "subscription_sharing_usage_limit_exceeded")
        {
            Some(FailureReason::RateLimited)
        } else {
            None
        },
        message,
        diagnostic: value.map(Value::encode),
        retry_after: None,
        charges: None,
    }
}

pub(super) fn sanitize(failure: &mut Failure, redactor: &Redactor) {
    fn field(value: &mut String, redactor: &Redactor) {
        *value = redactor.text(value);
        if value.len() > 8192 {
            let end = value.floor_char_boundary(8192);
            value.truncate(end);
            value.push_str(" [diagnostic truncated]");
        }
    }
    field(&mut failure.message, redactor);
    if let Some(diagnostic) = &mut failure.diagnostic {
        field(diagnostic, redactor);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn http_decoder_handles_split_headers_and_delivers_only_body() {
        let mut decoder = Headers::default();
        let mut seen = Vec::new();
        for chunk in b"HTTP/1.1 200 OK\r\nx-request-id: req_1\r\n\r\ndata: hi\n\n".chunks(5) {
            decoder
                .feed(chunk, &mut |body| {
                    seen.extend_from_slice(body);
                    Ok(())
                })
                .unwrap();
        }
        assert_eq!(seen, b"data: hi\n\n");
        assert_eq!(decoder.request_id.as_deref(), Some("req_1"));
    }

    #[test]
    fn structured_error_preserves_code_and_parameter_without_retrying_plan_limit() {
        let value = json::parse(r#"{"error":{"code":"subscription_sharing_usage_limit_exceeded","param":"model","message":"limit"}}"#).unwrap();
        let failure = failure(Some(&value), Some(429));
        assert_eq!(failure.kind, FailureKind::Terminal);
        assert!(failure.message.contains("HTTP 429"));
        assert!(failure.message.contains("param: model"));
        assert!(failure.message.contains("settings/usage"));
        assert!(failure.diagnostic.as_deref().unwrap().contains("\"error\""));
    }

    #[test]
    fn ordinary_rate_limit_is_temporary_without_plan_usage_hint() {
        let value =
            json::parse(r#"{"error":{"code":"rate_limit_exceeded","message":"slow down"}}"#)
                .unwrap();
        let failure = failure(Some(&value), Some(429));
        assert_eq!(failure.kind, FailureKind::Temporary);
        assert_eq!(failure.reason, Some(FailureReason::RateLimited));
        assert!(!failure.message.contains("settings/usage"));
    }

    #[test]
    fn transient_stream_codes_do_not_need_an_http_error_status() {
        for code in ["slow_down", "server_is_overloaded"] {
            let value = Value::object([("error", Value::object([("code", Value::string(code))]))]);
            let failure = failure(Some(&value), None);
            assert_eq!(failure.kind, FailureKind::Temporary);
            assert_eq!(
                failure.reason,
                (code == "slow_down").then_some(FailureReason::RateLimited)
            );
        }
    }

    #[test]
    fn diagnostic_is_redacted_before_it_is_bounded() {
        let secret = "secret-token-123";
        let mut failure = failure(
            Some(
                &json::parse(&format!(r#"{{"detail":"{}{}"}}"#, "x".repeat(8180), secret)).unwrap(),
            ),
            Some(403),
        );
        sanitize(&mut failure, &Redactor::new(secret.into()));
        assert!(!failure.message.contains(secret));
        assert!(!failure.diagnostic.as_deref().unwrap().contains(secret));
        assert!(
            failure
                .diagnostic
                .as_deref()
                .unwrap()
                .contains("[diagnostic truncated]")
        );
    }
}
