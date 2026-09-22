//! Account access through Jecode's authenticated TLS, with optional user JSON storage.
mod credentials;
mod login;
mod persistent;
mod reply;

use super::{HttpResponseStream, Limits, Progress, Request, Response, auth, encode_http};
use crate::tls::{Budget, Connection, NetworkError, trust::TrustStore};
use std::{
    fmt,
    ops::ControlFlow,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Network(NetworkError),
    Protocol(super::Error),
    Login(auth::Error),
    Http(crate::http::Error),
    Status(u16),
    Content,
    Trust,
    Expired,
    Storage,
    AccountChanged,
}
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Network(error) => error.fmt(f),
            Self::Protocol(super::Error::HttpStatus(status)) | Self::Status(status) => {
                write!(f, "account endpoint returned HTTP {status}")
            }
            Self::Protocol(error) => error.fmt(f),
            Self::Login(error) => error.fmt(f),
            Self::Http(error) => error.fmt(f),
            Self::Content => f.write_str("login endpoint did not return valid JSON content"),
            Self::Trust => f.write_str("native certificate trust data unavailable"),
            Self::Expired => f.write_str("account access expired; restart to sign in again"),
            Self::Storage => f.write_str("cannot read or update ~/.jecode/v1/credentials.json; check permissions and JSON format"),
            Self::AccountChanged => f.write_str("the saved account changed or signed out; restart to continue"),
        }
    }
}
impl std::error::Error for Error {}
impl From<NetworkError> for Error {
    fn from(value: NetworkError) -> Self {
        Self::Network(value)
    }
}
impl From<super::Error> for Error {
    fn from(value: super::Error) -> Self {
        Self::Protocol(value)
    }
}
impl From<auth::Error> for Error {
    fn from(value: auth::Error) -> Self {
        Self::Login(value)
    }
}
impl From<crate::http::Error> for Error {
    fn from(value: crate::http::Error) -> Self {
        Self::Http(value)
    }
}

pub struct Client {
    trust: TrustStore,
    tokens: auth::Tokens,
    store: Option<crate::state::Store>,
}
impl Client {
    /// Explicit device login. Does not discover, persist or refresh credentials.
    pub fn login(
        budget: &Budget<'_>,
        code: impl FnMut(&str) -> ControlFlow<()>,
    ) -> Result<Self, Error> {
        budget.check()?;
        let trust = TrustStore::native().map_err(|_| Error::Trust)?;
        let tokens = login::run(&trust, budget, code)?;
        Ok(Self {
            trust,
            tokens,
            store: None,
        })
    }

    /// Exactly one request; even a pre-output failure is never replayed here.
    pub fn generate(
        &mut self,
        request: &Request,
        budget: &Budget<'_>,
        progress: impl FnMut(Progress<'_>) -> ControlFlow<()>,
    ) -> Result<Response, Error> {
        budget.check()?;
        self.ensure_access(budget)?;
        if unix_seconds()? >= self.tokens.expires_at().saturating_sub(30) {
            return Err(Error::Expired);
        }
        let bytes = encode_http(
            request,
            self.tokens.access_token(),
            self.tokens.account_id(),
        )?;
        let connect = Budget {
            deadline: budget
                .deadline
                .min(std::time::Instant::now() + Duration::from_secs(30)),
            cancelled: budget.cancelled,
        };
        let mut connection = Connection::connect("chatgpt.com", &self.trust, &connect)?;
        connection.write(&bytes, &connect)?;
        let mut response = HttpResponseStream::new(Limits {
            event_bytes: 1024 * 1024,
            output_bytes: 1024 * 1024,
            ..Limits::default()
        });
        let mut progress = progress;
        while !response.is_finished() {
            let Some(bytes) = connection.read(budget)? else {
                break;
            };
            response.push(&bytes.bytes, &mut progress)?;
        }
        let result = response.finish()?;
        // Completion is established by the model event. A best-effort close
        // cannot turn a completed generation into a retryable failure.
        let _ = connection.close(&Budget {
            deadline: budget
                .deadline
                .min(std::time::Instant::now() + Duration::from_millis(100)),
            cancelled: budget.cancelled,
        });
        Ok(result)
    }
}

fn unix_seconds() -> Result<u64, Error> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|v| v.as_secs())
        .map_err(|_| NetworkError::Clock.into())
}
