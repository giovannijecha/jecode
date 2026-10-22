//! Monotonic, bounded retries for background checkpoint collection.
use std::time::{Duration, Instant};

const INITIAL: Duration = Duration::from_secs(1);
const MAXIMUM: Duration = Duration::from_secs(60);

#[derive(Default)]
pub(crate) struct CollectionRetry {
    until: Option<Instant>,
    delay: Duration,
}

impl CollectionRetry {
    pub fn blocked(&self, now: Instant) -> bool {
        self.until.is_some_and(|until| now < until)
    }

    pub fn failed(&mut self, now: Instant) {
        self.delay = if self.delay.is_zero() {
            INITIAL
        } else {
            self.delay.saturating_mul(2).min(MAXIMUM)
        };
        self.until = Some(now + self.delay);
    }

    pub fn clear(&mut self) {
        self.delay = Duration::ZERO;
        self.until = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failed_collection_retry_grows_caps_and_resets() {
        let mut retry = CollectionRetry::default();
        let mut now = Instant::now();
        assert!(!retry.blocked(now));
        for seconds in [1, 2, 4, 8, 16, 32, 60, 60] {
            retry.failed(now);
            assert!(retry.blocked(now + Duration::from_secs(seconds) - Duration::from_millis(1)));
            now += Duration::from_secs(seconds);
            assert!(!retry.blocked(now));
        }
        retry.clear();
        assert!(!retry.blocked(now));
        retry.failed(now);
        assert!(retry.blocked(now + Duration::from_millis(999)));
        assert!(!retry.blocked(now + INITIAL));
    }
}
