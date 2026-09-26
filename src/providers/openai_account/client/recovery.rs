//! A generation request can be retried only before any application record is
//! accepted by the local socket. Local acceptance never proves remote receipt.
use super::{Error, RequestStage};
use crate::providers::openai_account::ProviderFailure;
use crate::tls::{Budget, NetworkError};
use std::{fmt, io, thread, time::Duration};

pub(super) const MAX_CONNECTION_ATTEMPTS: u8 = 3;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Termination {
    SetupTimeout,
    WriteTimeout,
    FirstResponseTimeout,
    IdleTimeout,
    TotalBudget,
    Cancelled,
}
impl Termination {
    pub fn name(self) -> &'static str {
        match self {
            Self::SetupTimeout => "setup_timeout",
            Self::WriteTimeout => "write_timeout",
            Self::FirstResponseTimeout => "first_response_timeout",
            Self::IdleTimeout => "stream_idle_timeout",
            Self::TotalBudget => "total_budget_expired",
            Self::Cancelled => "cancelled",
        }
    }
    pub fn parse(name: &str) -> Option<Self> {
        Some(match name {
            "setup_timeout" => Self::SetupTimeout,
            "write_timeout" => Self::WriteTimeout,
            "first_response_timeout" => Self::FirstResponseTimeout,
            "stream_idle_timeout" => Self::IdleTimeout,
            "total_budget_expired" => Self::TotalBudget,
            "cancelled" => Self::Cancelled,
            _ => return None,
        })
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Delivery {
    #[default]
    NotSubmitted,
    PossiblySubmitted,
    Streaming,
    Completed,
}
impl Delivery {
    pub fn name(self) -> &'static str {
        match self {
            Self::NotSubmitted => "not_submitted",
            Self::PossiblySubmitted => "possibly_submitted",
            Self::Streaming => "streaming_unvalidated",
            Self::Completed => "validated_completion",
        }
    }
    pub fn parse(name: &str) -> Option<Self> {
        Some(match name {
            "not_submitted" => Self::NotSubmitted,
            "possibly_submitted" => Self::PossiblySubmitted,
            "streaming_unvalidated" => Self::Streaming,
            "validated_completion" => Self::Completed,
            _ => return None,
        })
    }
}
impl fmt::Display for Delivery {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::NotSubmitted => "generation request was not submitted",
            Self::PossiblySubmitted => "generation request may have been submitted",
            Self::Streaming => "response began but has no validated completion",
            Self::Completed => "validated completion received",
        })
    }
}

/// Safe diagnostics only. No request or response material enters this record.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Attempt {
    /// One-based request number within this generation or compaction command;
    /// zero in older records. It is not a session-wide sequence.
    pub request_sequence: u32,
    /// One-based connection attempt for this request; zero in older records.
    pub connection_attempt: u8,
    pub delivery: Delivery,
    pub stage: Option<RequestStage>,
    /// Time in the failed stage; zero when there was no failed stage.
    pub stage_elapsed_ms: u64,
    /// Attempt age and time since the write or last accepted SSE event.
    pub request_elapsed_ms: u64,
    pub since_progress_ms: Option<u64>,
    pub termination: Option<Termination>,
    pub operation: Option<String>,
    pub category: Option<String>,
    pub os_code: Option<i32>,
    pub accepted_wire_bytes: usize,
    /// TLS bytes returned by local TCP reads after the request write, even if
    /// the final record was incomplete. This does not prove provider completion.
    pub received_wire_bytes: usize,
    /// Decrypted application bytes handed to HTTP framing.
    pub response_plaintext_bytes: usize,
    pub response_status: Option<u16>,
    /// Complete SSE data events delivered to the model decoder.
    pub stream_events: u32,
    pub provider_failure: Option<ProviderFailure>,
    pub diagnostic: Option<String>,
    pub retrying: bool,
}
#[derive(Clone, Copy, Default)]
pub(super) struct Trace {
    pub started: Option<std::time::Instant>,
    pub stage: Option<RequestStage>,
    pub stage_elapsed_ms: u64,
    pub request_elapsed_ms: u64,
    pub since_progress_ms: Option<u64>,
    pub termination: Option<Termination>,
    pub received_wire_bytes: usize,
    pub response_plaintext_bytes: usize,
    pub response_status: Option<u16>,
    pub stream_events: u32,
}
impl Attempt {
    pub(super) fn failed(
        error: Error,
        delivery: Delivery,
        accepted_wire_bytes: usize,
        retrying: bool,
        connection_attempt: u8,
        trace: Trace,
    ) -> Self {
        let mut attempt = Self {
            connection_attempt,
            delivery,
            accepted_wire_bytes,
            stage: trace.stage,
            stage_elapsed_ms: trace.stage_elapsed_ms,
            request_elapsed_ms: trace.request_elapsed_ms,
            since_progress_ms: trace.since_progress_ms,
            termination: trace.termination,
            received_wire_bytes: trace.received_wire_bytes,
            response_plaintext_bytes: trace.response_plaintext_bytes,
            response_status: trace.response_status,
            stream_events: trace.stream_events,
            diagnostic: Some(error.to_string()),
            retrying,
            ..Self::default()
        };
        if let Error::Transport { stage, error, .. } = error {
            attempt.stage = Some(stage);
            if error == NetworkError::Cancelled {
                attempt.termination = Some(Termination::Cancelled);
            }
            if let NetworkError::Io(failure) | NetworkError::Dns(failure) = error {
                attempt.operation = Some(failure.operation.to_string());
                attempt.category = Some(format!("{:?}", failure.kind));
                attempt.os_code = failure.os_code;
            } else if let NetworkError::Eof(operation) = error {
                attempt.operation = Some(operation.to_string());
                attempt.category = Some("UnexpectedEof".into());
            }
        } else if matches!(error, Error::Response { .. }) {
            attempt.stage = Some(RequestStage::ResponseRead);
            if let Error::Response {
                error: crate::providers::openai_account::Error::RemoteFailure(failure),
                ..
            } = error
            {
                attempt.provider_failure = Some(failure);
            }
            if matches!(
                error,
                Error::Response {
                    error: crate::providers::openai_account::Error::Cancelled,
                    ..
                }
            ) {
                attempt.termination = Some(Termination::Cancelled);
            }
        }
        attempt
    }
    pub(super) fn completed(
        accepted_wire_bytes: usize,
        connection_attempt: u8,
        trace: Trace,
    ) -> Self {
        Self {
            connection_attempt,
            delivery: Delivery::Completed,
            accepted_wire_bytes,
            received_wire_bytes: trace.received_wire_bytes,
            response_plaintext_bytes: trace.response_plaintext_bytes,
            response_status: trace.response_status,
            stream_events: trace.stream_events,
            request_elapsed_ms: trace.request_elapsed_ms,
            ..Self::default()
        }
    }
}

pub(super) fn transient(error: &NetworkError) -> bool {
    match error {
        NetworkError::Timeout | NetworkError::Eof(_) => true,
        NetworkError::Io(failure) | NetworkError::Dns(failure) => matches!(
            failure.kind,
            io::ErrorKind::ConnectionRefused
                | io::ErrorKind::ConnectionReset
                | io::ErrorKind::ConnectionAborted
                | io::ErrorKind::BrokenPipe
                | io::ErrorKind::TimedOut
                | io::ErrorKind::WouldBlock
                | io::ErrorKind::Interrupted
        ),
        _ => false,
    }
}
pub(super) fn can_retry(error: Error, attempt: u8, budget: &Budget<'_>) -> bool {
    attempt < MAX_CONNECTION_ATTEMPTS
        && budget.check().is_ok()
        && matches!(error, Error::Transport { delivery: Delivery::NotSubmitted, error, .. } if transient(&error))
}
pub(super) fn backoff(attempt: u8, budget: &Budget<'_>) -> Result<(), NetworkError> {
    let delay = Duration::from_millis(u64::from(attempt) * 50);
    let until = std::time::Instant::now() + delay;
    loop {
        budget.check()?;
        let remaining = until.saturating_duration_since(std::time::Instant::now());
        if remaining.is_zero() {
            return Ok(());
        }
        thread::sleep(remaining.min(Duration::from_millis(10)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
            mpsc,
        },
        time::Instant,
    };

    #[test]
    fn cancellation_interrupts_an_active_backoff() {
        let cancelled = Arc::new(AtomicBool::new(false));
        let signal = Arc::clone(&cancelled);
        let (started, ready) = mpsc::channel();
        let worker = thread::spawn(move || {
            started.send(()).unwrap();
            thread::sleep(Duration::from_millis(20));
            signal.store(true, Ordering::Release);
        });
        ready.recv().unwrap();
        let budget = Budget {
            cancelled: &cancelled,
            deadline: Some(Instant::now() + Duration::from_secs(1)),
        };
        assert_eq!(backoff(3, &budget), Err(NetworkError::Cancelled));
        worker.join().unwrap();
    }
}
