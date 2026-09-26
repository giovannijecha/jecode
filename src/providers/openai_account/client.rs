//! Account access through Jecode's authenticated TLS, with optional user JSON storage.
mod catalog;
#[cfg(all(test, any(windows, target_os = "linux")))]
mod coordination_tests;
mod credentials;
mod exchange;
mod login;
mod persistent;
mod recovery;
mod reply;
mod timing;

use super::{Progress, Request, Response, auth, encode_http};
use crate::tls::{ApplicationWrite, Budget, Connection, NetworkError, trust::TrustStore};
#[cfg(test)]
use exchange::exchange;
use exchange::{ExchangeProgress, ResponseChannel, exchange_with_idle};
use std::{
    fmt,
    ops::ControlFlow,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Network(NetworkError),
    Transport {
        stage: RequestStage,
        error: NetworkError,
        delivery: Delivery,
        accepted_wire_bytes: usize,
    },
    Protocol(super::Error),
    Response {
        error: super::Error,
        delivery: Delivery,
    },
    Login(auth::Error),
    Http(crate::http::Error),
    Status(u16),
    Content,
    Catalog(super::catalog::Error),
    Trust,
    Expired,
    Storage,
    AccountChanged,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RequestStage {
    Connect,
    RequestWrite,
    ResponseRead,
}
pub use recovery::{Attempt, Delivery, Termination};
pub const DEFAULT_STREAM_IDLE_TIMEOUT_MS: u64 = timing::DEFAULT_STREAM_IDLE_TIMEOUT_MS;
pub const MIN_STREAM_IDLE_TIMEOUT_MS: u64 = timing::MIN_STREAM_IDLE_TIMEOUT_MS;
pub const MAX_STREAM_IDLE_TIMEOUT_MS: u64 = timing::MAX_STREAM_IDLE_TIMEOUT_MS;
impl fmt::Display for RequestStage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Connect => "connection setup",
            Self::RequestWrite => "request write",
            Self::ResponseRead => "response read",
        })
    }
}
impl RequestStage {
    pub fn name(self) -> &'static str {
        match self {
            Self::Connect => "connect",
            Self::RequestWrite => "request_write",
            Self::ResponseRead => "response_read",
        }
    }
    pub fn parse(name: &str) -> Option<Self> {
        Some(match name {
            "connect" => Self::Connect,
            "request_write" => Self::RequestWrite,
            "response_read" => Self::ResponseRead,
            _ => return None,
        })
    }
}
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Network(error) => error.fmt(f),
            Self::Transport { stage, error, delivery, accepted_wire_bytes } => {
                write!(f, "{error} / {stage} / {delivery}")?;
                if *accepted_wire_bytes != 0 {
                    write!(f, " / {accepted_wire_bytes} TLS application wire bytes accepted locally")?;
                }
                if matches!(delivery, Delivery::PossiblySubmitted | Delivery::Streaming) {
                    f.write_str(" / remote outcome uncertain; send an explicit continuation to proceed from the recorded state")?;
                }
                Ok(())
            }
            Self::Protocol(super::Error::HttpStatus(status)) | Self::Status(status) => {
                write!(f, "account endpoint returned HTTP {status}")
            }
            Self::Response { error: super::Error::HttpStatus(status), .. } => {
                write!(f, "account endpoint returned HTTP {status}")?;
                if *status == 401 { f.write_str("; use /logout then /login to sign in again")?; }
                Ok(())
            }
            Self::Response { error, delivery } => {
                write!(f, "{error} / {delivery}")?;
                if !matches!(error, super::Error::RemoteFailure | super::Error::Cancelled) {
                    f.write_str(" / completion is unvalidated; send an explicit continuation to use the recorded state")?;
                }
                Ok(())
            }
            Self::Protocol(error) => error.fmt(f),
            Self::Login(error) => error.fmt(f),
            Self::Http(error) => error.fmt(f),
            Self::Content => f.write_str("account endpoint did not return valid JSON content"),
            Self::Catalog(_) => f.write_str("account model catalog is empty, malformed or oversized"),
            Self::Trust => f.write_str("native certificate trust data unavailable"),
            Self::Expired => f.write_str("account access expired; use /login or jecode login to sign in again"),
            Self::Storage => f.write_str("cannot read or update ~/.jecode/v1/credentials.json; check permissions, JSON format and other Jecode instances, then retry"),
            Self::AccountChanged => f.write_str("the saved account changed or signed out; sign in again with /login or jecode login"),
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
    catalog: Option<(std::time::Instant, super::catalog::Catalog)>,
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
            catalog: None,
        })
    }

    /// A fresh connection may be retried only if application submission is
    /// known not to have occurred. Ambiguous sends and streams are never replayed.
    pub fn generate(
        &mut self,
        request: &Request,
        budget: &Budget<'_>,
        progress: impl FnMut(Progress<'_>) -> ControlFlow<()>,
    ) -> Result<Response, Error> {
        self.generate_with_idle(
            request,
            budget,
            Duration::from_millis(timing::DEFAULT_STREAM_IDLE_TIMEOUT_MS),
            progress,
        )
    }

    pub fn generate_with_idle(
        &mut self,
        request: &Request,
        budget: &Budget<'_>,
        idle_timeout: Duration,
        progress: impl FnMut(Progress<'_>) -> ControlFlow<()>,
    ) -> Result<Response, Error> {
        self.generate_with_policy(
            request,
            budget,
            progress,
            |trust, connect| Connection::connect("chatgpt.com", trust, connect),
            timing::ATTEMPT_STAGE_TIMEOUT,
            idle_timeout,
        )
    }

    #[cfg(test)]
    fn generate_with<C: ResponseChannel>(
        &mut self,
        request: &Request,
        budget: &Budget<'_>,
        progress: impl FnMut(Progress<'_>) -> ControlFlow<()>,
        connect_channel: impl FnMut(&TrustStore, &Budget<'_>) -> Result<C, NetworkError>,
    ) -> Result<Response, Error> {
        self.generate_with_policy(
            request,
            budget,
            progress,
            connect_channel,
            Duration::from_secs(30),
            Duration::from_millis(timing::DEFAULT_STREAM_IDLE_TIMEOUT_MS),
        )
    }

    #[cfg(test)]
    fn generate_with_timing<C: ResponseChannel>(
        &mut self,
        request: &Request,
        budget: &Budget<'_>,
        progress: impl FnMut(Progress<'_>) -> ControlFlow<()>,
        connect_channel: impl FnMut(&TrustStore, &Budget<'_>) -> Result<C, NetworkError>,
        attempt_timeout: Duration,
    ) -> Result<Response, Error> {
        self.generate_with_policy(
            request,
            budget,
            progress,
            connect_channel,
            attempt_timeout,
            Duration::from_millis(timing::DEFAULT_STREAM_IDLE_TIMEOUT_MS),
        )
    }

    fn generate_with_policy<C: ResponseChannel>(
        &mut self,
        request: &Request,
        budget: &Budget<'_>,
        progress: impl FnMut(Progress<'_>) -> ControlFlow<()>,
        mut connect_channel: impl FnMut(&TrustStore, &Budget<'_>) -> Result<C, NetworkError>,
        attempt_timeout: Duration,
        idle_timeout: Duration,
    ) -> Result<Response, Error> {
        budget.check()?;
        let mut progress = progress;
        for attempt in 1..=recovery::MAX_CONNECTION_ATTEMPTS {
            budget.check()?;
            let setup_started = std::time::Instant::now();
            let connect = Budget {
                deadline: Some(timing::stage_deadline(
                    budget,
                    setup_started,
                    attempt_timeout,
                )),
                cancelled: budget.cancelled,
            };
            let mut delivery = Delivery::NotSubmitted;
            let mut write = ApplicationWrite::default();
            let mut trace = recovery::Trace {
                started: Some(setup_started),
                ..Default::default()
            };
            let access_started = std::time::Instant::now();
            if let Err(error) = self.ensure_access(&connect) {
                if matches!(
                    error,
                    Error::Network(NetworkError::Timeout | NetworkError::Cancelled)
                ) {
                    record_setup_access_error(
                        &mut trace,
                        error,
                        budget,
                        setup_started,
                        access_started,
                    );
                    let _ = progress(Progress::Attempt(Attempt::failed(
                        error, delivery, 0, false, attempt, trace,
                    )));
                }
                return Err(error);
            }
            let result = (|| {
                let stage_started = std::time::Instant::now();
                let mut connection = connect_channel(&self.trust, &connect).map_err(|error| {
                    trace.stage_elapsed_ms = stage_started.elapsed().as_millis() as u64;
                    trace.request_elapsed_ms = setup_started.elapsed().as_millis() as u64;
                    trace.termination = timing::stage_termination(
                        error,
                        budget,
                        std::time::Instant::now(),
                        Termination::SetupTimeout,
                    );
                    Error::Transport {
                        stage: RequestStage::Connect,
                        error,
                        delivery,
                        accepted_wire_bytes: 0,
                    }
                })?;
                // The connection may have taken time while another instance
                // signed out or replaced the account. This is the last local
                // authorization before sending. It holds no response lease.
                let access_started = std::time::Instant::now();
                let post_connect = Budget {
                    deadline: Some(timing::stage_deadline(
                        budget,
                        access_started,
                        timing::ATTEMPT_STAGE_TIMEOUT,
                    )),
                    cancelled: budget.cancelled,
                };
                self.ensure_access(&post_connect).inspect_err(|error| {
                    record_setup_access_error(
                        &mut trace,
                        *error,
                        budget,
                        setup_started,
                        access_started,
                    );
                })?;
                if unix_seconds()? >= self.tokens.expires_at().saturating_sub(30) {
                    return Err(Error::Expired);
                }
                let bytes = encode_http(
                    request,
                    self.tokens.access_token(),
                    self.tokens.account_id(),
                )?;
                // The setup budget has served its purpose. A later account
                // check cannot consume the write's fresh 30-second allowance.
                let write_budget = Budget {
                    deadline: Some(timing::stage_deadline(
                        budget,
                        std::time::Instant::now(),
                        attempt_timeout,
                    )),
                    cancelled: budget.cancelled,
                };
                exchange_with_idle(
                    &mut connection,
                    &bytes,
                    budget,
                    &write_budget,
                    idle_timeout,
                    ExchangeProgress {
                        delivery: &mut delivery,
                        write: &mut write,
                        trace: &mut trace,
                    },
                    &mut progress,
                )
            })();
            match result {
                Ok(response) => {
                    let _ = progress(Progress::Attempt(Attempt::completed(
                        write.accepted_wire_bytes,
                        attempt,
                        trace,
                    )));
                    return Ok(response);
                }
                Err(error) => {
                    let retrying = recovery::can_retry(error, attempt, budget);
                    if progress(Progress::Attempt(Attempt::failed(
                        error,
                        delivery,
                        write.accepted_wire_bytes,
                        retrying,
                        attempt,
                        trace,
                    )))
                    .is_break()
                    {
                        return Err(NetworkError::Cancelled.into());
                    }
                    if !retrying {
                        return Err(error);
                    }
                    recovery::backoff(attempt, budget)?;
                }
            }
        }
        unreachable!("bounded connection attempts return a result")
    }
}

fn record_setup_access_error(
    trace: &mut recovery::Trace,
    error: Error,
    budget: &Budget<'_>,
    started: std::time::Instant,
    stage_started: std::time::Instant,
) {
    let now = std::time::Instant::now();
    trace.stage = Some(RequestStage::Connect);
    trace.stage_elapsed_ms = stage_started
        .elapsed()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX);
    trace.request_elapsed_ms = started.elapsed().as_millis().try_into().unwrap_or(u64::MAX);
    if let Error::Network(network) = error {
        trace.termination =
            timing::stage_termination(network, budget, now, Termination::SetupTimeout);
    }
}

fn unix_seconds() -> Result<u64, Error> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|v| v.as_secs())
        .map_err(|_| NetworkError::Clock.into())
}

#[cfg(test)]
#[path = "client/tests.rs"]
mod tests;
