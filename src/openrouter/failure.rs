use crate::json::Value;
use std::time::Duration;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Kind {
    Temporary,
    Empty,
    Context,
    Length,
    Terminal,
}

#[derive(Clone, Debug)]
pub struct Failure {
    pub kind: Kind,
    pub message: String,
    pub retry_after: Option<Duration>,
}

impl Failure {
    pub fn temporary(message: impl Into<String>) -> Self {
        Self {
            kind: Kind::Temporary,
            message: message.into(),
            retry_after: None,
        }
    }
    pub fn response(message: impl Into<String>) -> Self {
        let message = message.into();
        if message.contains("stream ended before [DONE]") {
            Self::temporary(message)
        } else if message == "OpenRouter returned neither text nor executable tool calls" {
            Self {
                kind: Kind::Empty,
                message,
                retry_after: None,
            }
        } else if message.contains("finish_reason: length") {
            Self {
                kind: Kind::Length,
                message,
                retry_after: None,
            }
        } else {
            Self::from(message)
        }
    }
    pub fn api(value: &Value, status: u16) -> Self {
        let (detail, provider_marker) = provider_detail(value);
        let code = value.get("error").and_then(|error| error.get("code"));
        let status = code
            .and_then(Value::as_usize)
            .and_then(|code| u16::try_from(code).ok())
            .or_else(|| {
                code.and_then(Value::as_str)
                    .and_then(|code| code.parse().ok())
            })
            .unwrap_or(status);
        let marker = code
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_ascii_lowercase();
        let text = detail.to_ascii_lowercase();
        let markers = [marker.as_str(), provider_marker.as_str()];
        let context = markers.iter().any(|marker| {
            matches!(
                *marker,
                "context_length_exceeded" | "context_window_exceeded"
            )
        }) || ((status == 400 || status == 413 || status == 422)
            && (text.contains("context length")
                || text.contains("context window")
                || text.contains("maximum context")
                || text.contains("too many tokens")));
        let temporary = matches!(status, 408 | 425 | 429 | 500..=599)
            || markers.iter().any(|marker| {
                matches!(
                    *marker,
                    "server_error"
                        | "rate_limit_exceeded"
                        | "overloaded_error"
                        | "timeout"
                        | "provider_error"
                )
            });
        Self {
            kind: if context {
                Kind::Context
            } else if temporary && text.contains("empty response") {
                Kind::Empty
            } else if temporary {
                Kind::Temporary
            } else {
                Kind::Terminal
            },
            message: format!(
                "OpenRouter HTTP {status}: {detail}.{}",
                if status == 401 {
                    " Check your key with jecode setup."
                } else {
                    ""
                }
            ),
            retry_after: None,
        }
    }
}

impl super::Api {
    pub(super) fn failure(&self, value: &Value, status: u16) -> Failure {
        self.sanitize_failure(Failure::api(value, status))
    }

    pub(super) fn sanitize_failure(&self, mut failure: Failure) -> Failure {
        // Redact before clipping: otherwise a clipped key could retain a secret prefix.
        failure.message = self.redact(&failure.message);
        if failure.message.len() > 8192 {
            let mut end = 8192;
            while !failure.message.is_char_boundary(end) {
                end -= 1;
            }
            failure.message.truncate(end);
            failure.message.push_str(" [provider diagnostic truncated]");
        }
        failure
    }
}

fn provider_detail(value: &Value) -> (String, String) {
    let mut detail = super::api_error(value)
        .unwrap_or("Request failed")
        .to_owned();
    let metadata = value.get("error").and_then(|error| error.get("metadata"));
    let raw = metadata.and_then(|value| value.get("raw"));
    let parsed = raw
        .and_then(Value::as_str)
        .and_then(|text| crate::json::parse(text).ok());
    let underlying = parsed
        .as_ref()
        .or(raw)
        .map(|value| value.get("error").unwrap_or(value));
    let provider_marker = field(metadata, "provider_error_code")
        .or_else(|| field(underlying, "code"))
        .unwrap_or("")
        .to_ascii_lowercase();
    for (name, text) in [
        ("provider", field(metadata, "provider_name")),
        (
            "code",
            (!provider_marker.is_empty()).then_some(provider_marker.as_str()),
        ),
        ("parameter", field(underlying, "param")),
        ("detail", field(underlying, "message")),
    ] {
        if let Some(text) = text.filter(|text| !text.is_empty()) {
            detail.push_str(&format!("; {name}: {text}"));
        }
    }
    if parsed.is_none()
        && let Some(text) = raw
            .and_then(Value::as_str)
            .filter(|text| !text.trim().is_empty())
    {
        detail.push_str(&format!("; detail: {text}"));
    }
    (detail, provider_marker)
}

fn field<'a>(value: Option<&'a Value>, name: &str) -> Option<&'a str> {
    value
        .and_then(|value| value.get(name))
        .and_then(Value::as_str)
}
impl From<String> for Failure {
    fn from(message: String) -> Self {
        Self {
            kind: Kind::Terminal,
            message,
            retry_after: None,
        }
    }
}
impl From<&str> for Failure {
    fn from(message: &str) -> Self {
        message.to_string().into()
    }
}

#[cfg(test)]
mod tests;
