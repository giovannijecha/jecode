//! A generation request can be retried only before any application record is
//! accepted by the local socket. Local acceptance never proves remote receipt.
use super::{Error, RequestStage};
use crate::tls::{Budget, NetworkError};
use std::{fmt, io, thread, time::Duration};

pub(super) const MAX_CONNECTION_ATTEMPTS: u8 = 3;

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
    pub delivery: Delivery,
    pub stage: Option<RequestStage>,
    pub operation: Option<String>,
    pub category: Option<String>,
    pub os_code: Option<i32>,
    pub accepted_wire_bytes: usize,
    pub diagnostic: Option<String>,
    pub retrying: bool,
}
impl Attempt {
    pub(super) fn failed(
        error: Error,
        delivery: Delivery,
        accepted_wire_bytes: usize,
        retrying: bool,
    ) -> Self {
        let mut attempt = Self {
            delivery,
            accepted_wire_bytes,
            diagnostic: Some(error.to_string()),
            retrying,
            ..Self::default()
        };
        if let Error::Transport { stage, error, .. } = error {
            attempt.stage = Some(stage);
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
        }
        attempt
    }
    pub(super) fn completed(accepted_wire_bytes: usize) -> Self {
        Self {
            delivery: Delivery::Completed,
            accepted_wire_bytes,
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
            deadline: Instant::now() + Duration::from_secs(1),
        };
        assert_eq!(backoff(3, &budget), Err(NetworkError::Cancelled));
        worker.join().unwrap();
    }
}
