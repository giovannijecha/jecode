//! Native HTTP transport. Request details travel on stdin, never command arguments.

use crate::cancel::Cancellation;
use crate::process::{self, Output};
use std::process::Command;
use std::time::Duration;

pub(crate) struct Request<'a> {
    pub url: &'a str,
    pub method: &'a str,
    pub headers: &'a [(&'a str, &'a str)],
    pub body: Option<&'a [u8]>,
    pub timeout: Option<Duration>,
    pub idle: Option<Duration>,
    pub output_limit: usize,
}

pub(crate) fn run(
    request: Request<'_>,
    cancellation: &Cancellation,
    observer: &mut dyn FnMut(&[u8]) -> Result<(), String>,
) -> Result<Output, String> {
    let mut command = Command::new(crate::native::helper("http")?);
    crate::auth::isolate_environment(&mut command);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(crate::process::CREATE_NO_WINDOW);
    }
    let body = request.body.unwrap_or_default();
    let mut input = String::new();
    input.push_str(&encode(wire_url(request.url).as_bytes()));
    input.push('\n');
    input.push_str(&encode(request.method.as_bytes()));
    input.push('\n');
    // Zero means no total limit: streamed responses rely on the idle limit.
    let timeout = request
        .timeout
        .map_or(0, |timeout| timeout.as_millis().max(1));
    input.push_str(&timeout.to_string());
    input.push('\n');
    input.push_str(&request.headers.len().to_string());
    input.push('\n');
    for (name, value) in request.headers {
        input.push_str(&encode(name.as_bytes()));
        input.push('\n');
        input.push_str(&encode(value.as_bytes()));
        input.push('\n');
    }
    input.push_str(&body.len().to_string());
    input.push('\n');
    let mut input = input.into_bytes();
    input.extend_from_slice(body);
    // The native helper owns no descendants; the runner enforces cancellation
    // and inactivity while forwarding each response chunk to the caller.
    process::run_leaf(
        &mut command,
        Some(input),
        request.timeout,
        request.output_limit,
        cancellation,
        request.idle,
        &mut |bytes| observer(bytes),
    )
}

fn encode(bytes: &[u8]) -> String {
    crate::attachments::base64::encode(bytes)
}

/// Percent-encodes path and query bytes that some system URL parsers reject,
/// such as non-ASCII text or `|` before macOS 14. Valid `%XX` escapes stay.
fn wire_url(url: &str) -> String {
    let authority = url.find("://").map_or(0, |index| index + 3);
    let start = url[authority..]
        .find(['/', '?', '#'])
        .map_or(url.len(), |index| authority + index);
    let (origin, rest) = url.split_at(start);
    let bytes = rest.as_bytes();
    let mut wire = String::with_capacity(url.len());
    wire.push_str(origin);
    for (index, &byte) in bytes.iter().enumerate() {
        let escape = byte == b'%'
            && bytes
                .get(index + 1..index + 3)
                .is_some_and(|hex| hex.iter().all(u8::is_ascii_hexdigit));
        if byte.is_ascii_graphic() && !b"\"<>[\\]^`{|}".contains(&byte) && (byte != b'%' || escape)
        {
            wire.push(char::from(byte));
        } else {
            wire.push_str(&format!("%{byte:02X}"));
        }
    }
    wire
}

#[derive(Default)]
pub(crate) struct Decoder {
    pending: Vec<u8>,
    pub status: Option<u16>,
    pub headers: Vec<(String, String)>,
    ready: bool,
}

impl Decoder {
    pub fn feed(
        &mut self,
        bytes: &[u8],
        observer: &mut dyn FnMut(&[u8]) -> Result<(), String>,
    ) -> Result<(), String> {
        if self.ready {
            return observer(bytes);
        }
        self.pending.extend_from_slice(bytes);
        let Some(end) = self.pending.windows(4).position(|part| part == b"\r\n\r\n") else {
            if self.pending.len() > 64 * 1024 {
                return Err("HTTP response headers are too large".into());
            }
            return Ok(());
        };
        let header = std::str::from_utf8(&self.pending[..end])
            .map_err(|_| "Invalid HTTP response headers")?;
        let mut lines = header.split("\r\n");
        self.status = lines
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
            .and_then(|number| number.parse().ok());
        if self.status.is_none() {
            return Err("Invalid HTTP status".into());
        }
        for line in lines {
            let (name, value) = line.split_once(':').ok_or("Invalid HTTP header")?;
            self.headers
                .push((name.to_string(), value.trim().to_string()));
        }
        self.ready = true;
        let body = self.pending.split_off(end + 4);
        self.pending.clear();
        observer(&body)
    }

    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }
}

#[cfg(test)]
mod tests;
