use super::{
    Error, Head, chunk,
    head::{self, Body},
};
use std::ops::ControlFlow;

#[derive(Clone, Copy)]
pub struct Limits {
    pub header_bytes: usize,
    pub body_bytes: usize,
    pub framing_bytes: usize,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            header_bytes: 32 * 1024,
            body_bytes: 64 * 1024 * 1024,
            framing_bytes: 2 * 1024 * 1024,
        }
    }
}
pub enum Event<'a> {
    Head(&'a Head),
    Data(&'a [u8]),
    End,
}
#[derive(Clone, Copy)]
enum Phase {
    Head,
    Fixed(usize),
    Close,
    Size,
    Data(usize),
    DataEnd(usize),
    Trailer,
    Complete,
    Closed,
}

/// One response to a POST/GET. No pipelining, redirects, decompression or retries.
pub struct Decoder {
    limits: Limits,
    phase: Phase,
    buffer: Vec<u8>,
    body_bytes: usize,
    framing_bytes: usize,
    interim: usize,
    trailer_bytes: usize,
}
impl Default for Decoder {
    fn default() -> Self {
        Self::new(Limits::default())
    }
}
impl Decoder {
    pub fn new(limits: Limits) -> Self {
        Self {
            limits,
            phase: Phase::Head,
            buffer: Vec::new(),
            body_bytes: 0,
            framing_bytes: 0,
            interim: 0,
            trailer_bytes: 0,
        }
    }
    pub fn is_complete(&self) -> bool {
        matches!(self.phase, Phase::Complete)
    }
    pub fn push(
        &mut self,
        bytes: &[u8],
        mut callback: impl FnMut(Event<'_>) -> ControlFlow<()>,
    ) -> Result<(), Error> {
        let result = self.feed(bytes, &mut callback);
        if result.is_err() {
            self.close();
        }
        result
    }
    pub fn close(&mut self) {
        self.phase = Phase::Closed;
        self.buffer.clear();
    }
    pub fn finish(
        &mut self,
        mut callback: impl FnMut(Event<'_>) -> ControlFlow<()>,
    ) -> Result<(), Error> {
        let result = match self.phase {
            Phase::Complete => Ok(()),
            Phase::Close => self.end(&mut callback),
            Phase::Closed => Err(Error::Closed),
            _ => Err(Error::Truncated),
        };
        if result.is_err() {
            self.close();
        }
        result
    }
    fn end(
        &mut self,
        callback: &mut impl FnMut(Event<'_>) -> ControlFlow<()>,
    ) -> Result<(), Error> {
        self.phase = Phase::Complete;
        deliver(callback, Event::End)
    }
    fn feed(
        &mut self,
        mut bytes: &[u8],
        callback: &mut impl FnMut(Event<'_>) -> ControlFlow<()>,
    ) -> Result<(), Error> {
        if matches!(self.phase, Phase::Closed) {
            return Err(Error::Closed);
        }
        while !bytes.is_empty() {
            match self.phase {
                Phase::Complete => return Err(Error::ExtraData),
                Phase::Closed => return Err(Error::Closed),
                Phase::Fixed(left) | Phase::Data(left) => {
                    let count = left.min(bytes.len());
                    self.body(&bytes[..count], callback)?;
                    bytes = &bytes[count..];
                    self.phase = match self.phase {
                        Phase::Fixed(_) if left == count => {
                            self.end(callback)?;
                            Phase::Complete
                        }
                        Phase::Fixed(_) => Phase::Fixed(left - count),
                        _ if left == count => Phase::DataEnd(0),
                        _ => Phase::Data(left - count),
                    };
                }
                Phase::Close => {
                    self.body(bytes, callback)?;
                    bytes = &[];
                }
                Phase::Head | Phase::Size | Phase::Trailer | Phase::DataEnd(_) => {
                    let byte = bytes[0];
                    bytes = &bytes[1..];
                    self.framing_bytes = self.framing_bytes.checked_add(1).ok_or(Error::Limit)?;
                    if self.framing_bytes > self.limits.framing_bytes {
                        return Err(Error::Limit);
                    }
                    self.framing(byte, callback)?;
                }
            }
        }
        Ok(())
    }
    fn body(
        &mut self,
        bytes: &[u8],
        callback: &mut impl FnMut(Event<'_>) -> ControlFlow<()>,
    ) -> Result<(), Error> {
        if bytes.len() > self.limits.body_bytes.saturating_sub(self.body_bytes) {
            return Err(Error::Limit);
        }
        self.body_bytes += bytes.len();
        deliver(callback, Event::Data(bytes))
    }
    fn framing(
        &mut self,
        byte: u8,
        callback: &mut impl FnMut(Event<'_>) -> ControlFlow<()>,
    ) -> Result<(), Error> {
        if let Phase::DataEnd(at) = self.phase {
            if byte != b"\r\n"[at] {
                return Err(Error::Invalid);
            }
            self.phase = if at == 0 {
                Phase::DataEnd(1)
            } else {
                Phase::Size
            };
            return Ok(());
        }
        if byte == b'\n' && self.buffer.last() != Some(&b'\r')
            || self.buffer.last() == Some(&b'\r') && byte != b'\n'
        {
            return Err(Error::Invalid);
        }
        self.buffer.push(byte);
        let maximum = if matches!(self.phase, Phase::Head) {
            self.limits.header_bytes
        } else {
            8192.min(self.limits.header_bytes)
        };
        if self.buffer.len() > maximum {
            return Err(Error::Limit);
        }
        if matches!(self.phase, Phase::Head) {
            if !self.buffer.ends_with(b"\r\n\r\n") {
                return Ok(());
            }
            let (head, body) = head::parse(&self.buffer)?;
            self.buffer.clear();
            if head.status < 200 {
                self.interim += 1;
                if self.interim > 8 {
                    return Err(Error::Limit);
                }
                return Ok(());
            }
            if let Body::Length(size) = body
                && size > self.limits.body_bytes
            {
                return Err(Error::Limit);
            }
            deliver(callback, Event::Head(&head))?;
            self.phase = match body {
                Body::Length(n) => Phase::Fixed(n),
                Body::Chunked => Phase::Size,
                Body::Close => Phase::Close,
            };
            if matches!(self.phase, Phase::Fixed(0)) {
                self.end(callback)?;
            }
        } else if self.buffer.ends_with(b"\r\n") {
            let line = &self.buffer[..self.buffer.len() - 2];
            match self.phase {
                Phase::Size => {
                    let size = chunk::size(line)?;
                    if size > self.limits.body_bytes.saturating_sub(self.body_bytes) {
                        return Err(Error::Limit);
                    }
                    self.phase = if size == 0 {
                        Phase::Trailer
                    } else {
                        Phase::Data(size)
                    };
                }
                Phase::Trailer => {
                    self.trailer_bytes += self.buffer.len();
                    if self.trailer_bytes > self.limits.header_bytes {
                        return Err(Error::Limit);
                    }
                    if line.is_empty() {
                        self.end(callback)?;
                    } else {
                        let line = std::str::from_utf8(line).map_err(|_| Error::Invalid)?;
                        let (key, _) = head::field(line)?;
                        if matches!(
                            key.as_str(),
                            "content-length"
                                | "transfer-encoding"
                                | "content-encoding"
                                | "content-type"
                                | "host"
                                | "authorization"
                                | "www-authenticate"
                                | "location"
                        ) {
                            return Err(Error::Invalid);
                        }
                    }
                }
                _ => return Err(Error::Invalid),
            }
            self.buffer.clear();
        }
        Ok(())
    }
}
fn deliver(
    callback: &mut impl FnMut(Event<'_>) -> ControlFlow<()>,
    event: Event<'_>,
) -> Result<(), Error> {
    if callback(event).is_break() {
        Err(Error::Stopped)
    } else {
        Ok(())
    }
}
