//! Server-sent event splitting shared by streaming completion and decoding.

use crate::json::{self, Value};

/// Splits SSE events at blank lines, resuming where the previous call stopped.
#[derive(Default)]
pub struct Splitter {
    event_start: usize,
    line_start: usize,
    scan: usize,
}

impl Splitter {
    /// Returns the field lines of the next complete event in `body`.
    pub fn next<'a>(&mut self, body: &'a [u8]) -> Option<&'a [u8]> {
        while let Some(offset) = body.get(self.scan..)?.iter().position(|b| *b == b'\n') {
            let end = self.scan + offset;
            let line = &body[self.line_start..end];
            self.scan = end + 1;
            if line.strip_suffix(b"\r").unwrap_or(line).is_empty() {
                let event = &body[self.event_start..self.line_start];
                self.event_start = self.scan;
                self.line_start = self.scan;
                return Some(event);
            }
            self.line_start = self.scan;
        }
        self.scan = body.len();
        None
    }

    /// Offset just past the last event returned.
    pub fn end(&self) -> usize {
        self.event_start
    }

    /// Unterminated bytes after the last complete event.
    pub fn rest<'a>(&self, body: &'a [u8]) -> &'a [u8] {
        body.get(self.event_start..).unwrap_or_default()
    }
}

/// The JSON-RPC response for `id` carried by one event, if it holds one.
/// Empty priming events, other messages, server requests and data that is
/// not JSON are not responses and are skipped.
pub fn response(event: &[u8], id: u64) -> Option<Value> {
    let mut data = Vec::new();
    for line in event.split(|byte| *byte == b'\n') {
        let line = line.strip_suffix(b"\r").unwrap_or(line);
        if let Some(value) = line.strip_prefix(b"data:") {
            if !data.is_empty() {
                data.push(b'\n');
            }
            data.extend_from_slice(value.strip_prefix(b" ").unwrap_or(value));
        }
    }
    let text = std::str::from_utf8(&data).ok()?;
    if text.trim().is_empty() {
        return None;
    }
    let value = json::parse(text).ok()?;
    (value.get("id") == Some(&Value::number(id))
        && value.get("method").is_none()
        && (value.get("result").is_some() || value.get("error").is_some()))
    .then_some(value)
}

#[derive(Default)]
pub struct SseScanner {
    events: Splitter,
}

impl SseScanner {
    /// Returns the byte boundary of a complete SSE event with the requested response.
    pub fn response_end(&mut self, body: &[u8], id: u64) -> Option<usize> {
        while let Some(event) = self.events.next(body) {
            if response(event, id).is_some() {
                return Some(self.events.end());
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scanner_skips_priming_requests_and_noise_before_the_response() {
        let body = b"id: 7\r\ndata: \r\n\r\ndata: not json\n\ndata: {\"jsonrpc\":\"2.0\",\"id\":3,\"method\":\"ping\"}\n\ndata: {\"jsonrpc\":\"2.0\",\"id\":3,\"result\":{}}\n\ntrailing";
        let mut scanner = SseScanner::default();
        let half = body.len() / 2;
        assert_eq!(scanner.response_end(&body[..half], 3), None);
        let end = scanner.response_end(body, 3).unwrap();
        assert_eq!(&body[end..], b"trailing");
    }
}
