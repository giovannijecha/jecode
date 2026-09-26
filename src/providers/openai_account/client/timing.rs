//! One timing policy for every model request. Only complete, accepted SSE data
//! events renew response inactivity; bytes, headers and comments do not.
use super::recovery::Termination;
use crate::tls::{Budget, NetworkError};
use std::time::{Duration, Instant};

pub const DEFAULT_STREAM_IDLE_TIMEOUT_MS: u64 = 300_000;
pub const MIN_STREAM_IDLE_TIMEOUT_MS: u64 = 1_000;
pub const MAX_STREAM_IDLE_TIMEOUT_MS: u64 = 900_000;
pub const ATTEMPT_STAGE_TIMEOUT: Duration = Duration::from_secs(30);

pub fn stage_deadline(parent: &Budget<'_>, now: Instant, timeout: Duration) -> Instant {
    let stage = now + timeout;
    parent.deadline.map_or(stage, |total| total.min(stage))
}

pub fn stage_termination(
    error: NetworkError,
    parent: &Budget<'_>,
    now: Instant,
    stage: Termination,
) -> Option<Termination> {
    match error {
        NetworkError::Cancelled => Some(Termination::Cancelled),
        NetworkError::Timeout if parent.deadline.is_some_and(|total| now >= total) => {
            Some(Termination::TotalBudget)
        }
        NetworkError::Timeout => Some(stage),
        _ => None,
    }
}

/// Response waiting begins after the write, including headers and the first
/// complete SSE data event. Later accepted events establish renewable idle time.
pub struct ResponseWindow {
    started: Instant,
    last_event: Option<Instant>,
    idle: Duration,
}
impl ResponseWindow {
    pub fn new(started: Instant, idle: Duration) -> Self {
        Self {
            started,
            last_event: None,
            idle,
        }
    }
    pub fn since_progress(&self, now: Instant) -> u64 {
        now.saturating_duration_since(self.last_event.unwrap_or(self.started))
            .as_millis()
            .try_into()
            .unwrap_or(u64::MAX)
    }
    pub fn deadline(&self, parent: &Budget<'_>) -> Instant {
        stage_deadline(parent, self.last_event.unwrap_or(self.started), self.idle)
    }
    pub fn check(&self, parent: &Budget<'_>, now: Instant) -> Result<(), Termination> {
        if parent.cancelled.load(std::sync::atomic::Ordering::Acquire) {
            return Err(Termination::Cancelled);
        }
        if parent.deadline.is_some_and(|total| now >= total) {
            return Err(Termination::TotalBudget);
        }
        if now >= self.deadline(parent) {
            return Err(self.inactivity_kind());
        }
        Ok(())
    }
    pub fn inactivity_kind(&self) -> Termination {
        if self.last_event.is_some() {
            Termination::IdleTimeout
        } else {
            Termination::FirstResponseTimeout
        }
    }
    pub fn observed(&mut self, before: u32, after: u32, now: Instant) {
        if after > before {
            self.last_event = Some(now);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicBool;

    #[test]
    fn complete_events_renew_without_a_total_age_limit() {
        let origin = Instant::now();
        let cancelled = AtomicBool::new(false);
        let parent = Budget {
            deadline: None,
            cancelled: &cancelled,
        };
        let mut window = ResponseWindow::new(origin, Duration::from_secs(100));
        for n in 1..=12 {
            let now = origin + Duration::from_secs(n * 80);
            assert_eq!(window.check(&parent, now), Ok(()));
            window.observed((n - 1) as u32, n as u32, now);
        }
        assert_eq!(
            window.check(&parent, origin + Duration::from_secs(1_050)),
            Ok(())
        );
        assert_eq!(
            window.check(&parent, origin + Duration::from_secs(1_061)),
            Err(Termination::IdleTimeout)
        );
    }

    #[test]
    fn first_wait_and_explicit_total_are_distinct() {
        let origin = Instant::now();
        let cancelled = AtomicBool::new(false);
        let parent = Budget {
            deadline: None,
            cancelled: &cancelled,
        };
        let window = ResponseWindow::new(origin, Duration::from_secs(10));
        assert_eq!(
            window.check(&parent, origin + Duration::from_secs(10)),
            Err(Termination::FirstResponseTimeout)
        );
        let total = Budget {
            deadline: Some(origin + Duration::from_secs(7)),
            cancelled: &cancelled,
        };
        assert_eq!(
            window.check(&total, origin + Duration::from_secs(7)),
            Err(Termination::TotalBudget)
        );
        cancelled.store(true, std::sync::atomic::Ordering::Release);
        assert_eq!(
            window.check(&total, origin + Duration::from_secs(8)),
            Err(Termination::Cancelled)
        );
    }
}
