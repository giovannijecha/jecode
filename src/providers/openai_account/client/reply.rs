//! Bounded authentication JSON over verified HTTP; never expose a response body.
use super::Error;
use crate::http::{self, Event};
use std::ops::ControlFlow;

pub(super) struct Reply {
    decoder: http::Decoder,
    pub status: u16,
    pub body: String,
    bytes: Vec<u8>,
    error: Option<Error>,
}
impl Reply {
    pub fn new() -> Self {
        Self {
            decoder: http::Decoder::new(http::Limits {
                body_bytes: 65_536,
                ..Default::default()
            }),
            status: 0,
            body: String::new(),
            bytes: Vec::new(),
            error: None,
        }
    }
    pub fn is_complete(&self) -> bool {
        self.decoder.is_complete()
    }
    pub fn push(&mut self, bytes: &[u8]) -> Result<(), Error> {
        let Self {
            decoder,
            status,
            bytes: body,
            error,
            ..
        } = self;
        let result = decoder.push(bytes, |event| {
            match event {
                Event::Head(head) => {
                    *status = head.status;
                    if (300..400).contains(status) {
                        *error = Some(Error::Status(*status));
                    } else if !head.get("content-type").is_some_and(|value| {
                        value
                            .split(';')
                            .next()
                            .unwrap_or("")
                            .trim()
                            .eq_ignore_ascii_case("application/json")
                    }) {
                        *error = Some(if *status == 200 {
                            Error::Content
                        } else {
                            Error::Status(*status)
                        });
                    }
                }
                Event::Data(bytes) => body.extend_from_slice(bytes),
                Event::End => {}
            }
            if error.is_some() {
                ControlFlow::Break(())
            } else {
                ControlFlow::Continue(())
            }
        });
        if let Some(error) = self.error {
            return Err(error);
        }
        Ok(result?)
    }
    pub fn eof(&mut self) -> Result<(), Error> {
        Ok(self.decoder.finish(|_| ControlFlow::Continue(()))?)
    }
    pub fn finish(mut self) -> Result<Self, Error> {
        if let Some(error) = self.error {
            return Err(error);
        }
        if !self.is_complete() {
            return Err(http::Error::Truncated.into());
        }
        self.body =
            String::from_utf8(std::mem::take(&mut self.bytes)).map_err(|_| Error::Content)?;
        Ok(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn auth_accepts_only_complete_bounded_json_and_does_not_follow_redirects() {
        let wire = b"HTTP/1.1 200 OK\r\nContent-Type: application/json; charset=utf-8\r\nTransfer-Encoding: chunked\r\n\r\n2\r\n{}\r\n0\r\n\r\n";
        for split in 0..wire.len() {
            let mut reply = Reply::new();
            reply.push(&wire[..split]).unwrap();
            reply.push(&wire[split..]).unwrap();
            assert_eq!(reply.finish().unwrap().body, "{}");
        }
        for (wire, expected) in [
            (
                "HTTP/1.1 302 Redirect\r\nLocation: https://elsewhere.invalid\r\nContent-Length: 0\r\n\r\n",
                Error::Status(302),
            ),
            (
                "HTTP/1.1 403 Blocked\r\nContent-Type: text/html\r\nContent-Length: 6\r\n\r\nSECRET",
                Error::Status(403),
            ),
            (
                "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 0\r\n\r\n",
                Error::Content,
            ),
            (
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 65537\r\n\r\n",
                Error::Http(http::Error::Limit),
            ),
        ] {
            let mut reply = Reply::new();
            assert_eq!(reply.push(wire.as_bytes()), Err(expected));
            assert!(!expected.to_string().contains("SECRET"));
        }
        let mut reply = Reply::new();
        reply
            .push(
                b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 2\r\n\r\n{",
            )
            .unwrap();
        assert!(reply.eof().is_err());
        assert!(reply.finish().is_err());
    }
}
