//! Bounded UTF-8 SSE data decoder. No network, model protocol or JSON parsing.
//!
//! Events are delivered immediately to a borrowed callback, never accumulated
//! in an unbounded output queue. A callback break or malformed stream closes the
//! decoder permanently. Reconnection must use a new decoder and explicit policy.

use std::{fmt, ops::ControlFlow};

#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub line_bytes: usize,
    pub event_bytes: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            line_bytes: 64 * 1024,
            event_bytes: 1024 * 1024,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    LineTooLong,
    EventTooLarge,
    InvalidUtf8,
    Cancelled,
    Closed,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::LineTooLong => "stream line exceeds its byte limit",
            Self::EventTooLarge => "stream event exceeds its byte limit",
            Self::InvalidUtf8 => "stream contains invalid UTF-8",
            Self::Cancelled => "stream consumption was cancelled",
            Self::Closed => "stream decoder is closed",
        })
    }
}

impl std::error::Error for Error {}

pub struct Decoder {
    limits: Limits,
    line: Vec<u8>,
    data: String,
    has_data: bool,
    first_line: bool,
    skip_lf: bool,
    closed: bool,
}

impl Decoder {
    pub fn new(limits: Limits) -> Self {
        Self {
            limits,
            line: Vec::new(),
            data: String::new(),
            has_data: false,
            first_line: true,
            skip_lf: false,
            closed: false,
        }
    }

    /// Consume arbitrary byte chunks, including fragmented UTF-8 and CRLF.
    /// Completed events delivered before an error remain delivered; callers must
    /// not retry this generation as if nothing had happened.
    pub fn push(
        &mut self,
        bytes: &[u8],
        mut event: impl FnMut(&str) -> ControlFlow<()>,
    ) -> Result<(), Error> {
        if self.closed {
            return Err(Error::Closed);
        }
        let result = self.consume(bytes, &mut event);
        if result.is_err() {
            self.close();
        }
        result
    }

    /// Discard any incomplete event. EOF is not proof that a model response
    /// completed; the provider adapter must observe its own terminal message.
    pub fn close(&mut self) {
        self.closed = true;
        self.line.clear();
        self.data.clear();
        self.has_data = false;
    }

    fn consume(
        &mut self,
        bytes: &[u8],
        event: &mut impl FnMut(&str) -> ControlFlow<()>,
    ) -> Result<(), Error> {
        for &byte in bytes {
            if self.skip_lf {
                self.skip_lf = false;
                if byte == b'\n' {
                    continue;
                }
            }
            match byte {
                b'\r' | b'\n' => {
                    self.finish_line(event)?;
                    self.skip_lf = byte == b'\r';
                }
                _ => {
                    if self.line.len() >= self.limits.line_bytes {
                        return Err(Error::LineTooLong);
                    }
                    self.line.push(byte);
                }
            }
        }
        Ok(())
    }

    fn finish_line(
        &mut self,
        event: &mut impl FnMut(&str) -> ControlFlow<()>,
    ) -> Result<(), Error> {
        let line = std::str::from_utf8(&self.line).map_err(|_| Error::InvalidUtf8)?;
        let line = if self.first_line {
            self.first_line = false;
            line.strip_prefix('\u{feff}').unwrap_or(line)
        } else {
            line
        };
        if line.is_empty() {
            if self.has_data {
                let decision = event(&self.data);
                self.data.clear();
                self.has_data = false;
                if decision.is_break() {
                    return Err(Error::Cancelled);
                }
            }
        } else {
            let (field, value) = line.split_once(':').unwrap_or((line, ""));
            if field == "data" {
                let value = value.strip_prefix(' ').unwrap_or(value);
                let additional = value
                    .len()
                    .checked_add(usize::from(self.has_data))
                    .ok_or(Error::EventTooLarge)?;
                if additional > self.limits.event_bytes.saturating_sub(self.data.len()) {
                    return Err(Error::EventTooLarge);
                }
                if self.has_data {
                    self.data.push('\n');
                }
                self.data.push_str(value);
                self.has_data = true;
            }
        }
        self.line.clear();
        Ok(())
    }
}
