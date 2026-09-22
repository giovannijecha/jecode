//! Owned HTTP/1.1 wire framing. This module opens no connection and provides no TLS.
mod chunk;
mod head;
mod request;
mod response;

pub use head::Head;
pub use request::{post_form, post_json};
pub use response::{Decoder, Event, Limits};
use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Invalid,
    AmbiguousLength,
    Unsupported,
    Limit,
    Truncated,
    ExtraData,
    Stopped,
    Closed,
}
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Invalid => "invalid HTTP message",
            Self::AmbiguousLength => "ambiguous HTTP body framing",
            Self::Unsupported => "unsupported HTTP encoding or upgrade",
            Self::Limit => "HTTP message exceeds its configured limit",
            Self::Truncated => "HTTP response ended before its framing completed",
            Self::ExtraData => "unexpected bytes after HTTP response",
            Self::Stopped => "HTTP delivery was stopped by its consumer",
            Self::Closed => "HTTP decoder is closed",
        })
    }
}
impl std::error::Error for Error {}

fn token(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&byte)
}
