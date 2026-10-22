//! Persistent timestamps use UTC wall time, including time spent suspended.
//! Clamp backward samples within a process; retention across processes still
//! requires a correct system clock. Local waits use `Instant` separately.
use std::sync::atomic::{AtomicU64, Ordering};

static LAST: AtomicU64 = AtomicU64::new(0);

fn sampled(last: &AtomicU64, wall: impl FnOnce() -> u64) -> u64 {
    let now = wall();
    last.fetch_max(now, Ordering::Relaxed).max(now)
}

pub fn now() -> u64 {
    sampled(&LAST, crate::sessions::now)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn persisted_time_includes_a_suspended_interval() {
        let anchor = AtomicU64::new(0);
        let started = 1_000;
        sampled(&anchor, || started);
        let resumed_wall = started + 48 * 60 * 60 * 1_000;
        let resumed = sampled(&anchor, || resumed_wall);
        assert!(
            resumed >= resumed_wall,
            "persisted timestamps lost the suspension"
        );
    }

    #[test]
    fn new_process_sees_a_fresh_accept_as_unexpired() {
        const DAY: u64 = 24 * 60 * 60 * 1_000;
        let running = AtomicU64::new(0);
        sampled(&running, || 1_000);
        let resumed_wall = 1_000 + 2 * DAY;
        let accepted_until = sampled(&running, || resumed_wall) + DAY;
        let restarted = AtomicU64::new(0);
        let restarted_now = sampled(&restarted, || resumed_wall);
        assert!(
            restarted_now < accepted_until,
            "a fresh 24-hour accept was already expired"
        );
    }

    #[test]
    fn backward_wall_clock_samples_do_not_decrease_persisted_time() {
        let last = AtomicU64::new(0);
        assert_eq!(sampled(&last, || 1234), 1234);
        assert_eq!(sampled(&last, || 1000), 1234);
        assert_eq!(sampled(&last, || 2000), 2000);
    }
}
