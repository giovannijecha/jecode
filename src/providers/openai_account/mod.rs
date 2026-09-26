//! Experimental account Responses codec and connected device-login client.

pub mod auth;
pub mod catalog;
pub mod client;
#[cfg(test)]
mod continuation_tests;
mod events;
mod http;
mod request;
mod response;
mod snapshot;

pub use events::{Limits, Progress, ResponseStream};
pub use http::{HttpResponseStream, encode_http};
pub use request::{ENDPOINT, Input, Request, Tool};
pub use response::{Response, Status, ToolCall, Usage};

use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Json(crate::json::Error),
    Framing(crate::stream::Error),
    InvalidRequest,
    InvalidEvent,
    ConflictingOutput,
    InvalidTool,
    InvalidUsage,
    RemoteFailure,
    MissingTerminal,
    Limit,
    Cancelled,
    Closed,
    Http(crate::http::Error),
    HttpStatus(u16),
    ContentType(ContentKind),
}

/// Allowlisted response metadata only; never retain a provider header or body.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContentKind {
    Json,
    Html,
    Text,
    Other,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Provider payloads and credentials must never become error messages.
        f.write_str(match self {
            Self::Json(_) => "account response contains invalid or oversized JSON",
            Self::Framing(_) => "account response has invalid stream framing",
            Self::InvalidRequest => "account request is invalid",
            Self::InvalidEvent => "account response event is invalid",
            Self::ConflictingOutput => "account response output is inconsistent",
            Self::InvalidTool => "account response contains an invalid tool call",
            Self::InvalidUsage => "account response contains invalid token usage",
            Self::RemoteFailure => "account provider reported a failed response",
            Self::MissingTerminal => "account stream ended before a terminal response",
            Self::Limit => "account response exceeds its configured limit",
            Self::Cancelled => "account stream consumption was cancelled",
            Self::Closed => "account response stream is closed",
            Self::Http(_) => "account response has invalid HTTP framing",
            Self::HttpStatus(_) => "account provider returned an unsuccessful HTTP status",
            Self::ContentType(kind) => {
                let content = match kind {
                    ContentKind::Json => "JSON",
                    ContentKind::Html => "HTML",
                    ContentKind::Text => "plain text",
                    ContentKind::Other => "an unsupported Content-Type",
                };
                return write!(
                    f,
                    "account endpoint returned HTTP 200 with {content} instead of an event stream"
                );
            }
        })
    }
}
impl std::error::Error for Error {}
impl From<crate::json::Error> for Error {
    fn from(value: crate::json::Error) -> Self {
        Self::Json(value)
    }
}

fn field<'a>(value: &'a crate::json::Value, key: &str) -> Result<&'a str, Error> {
    value
        .get(key)
        .and_then(crate::json::Value::text)
        .ok_or(Error::InvalidEvent)
}

fn identifier(text: &str) -> bool {
    !text.is_empty() && text.len() <= 256 && !text.chars().any(char::is_control)
}
