//! Retention follows elapsed time within a process. A system-clock adjustment
//! must not immediately expire recovery data while Jecode is running. Across
//! process restarts, persisted deadlines still depend on the system clock.
use std::sync::OnceLock;
use std::time::Instant;

type Anchor = OnceLock<(u64, Instant)>;
static START: Anchor = OnceLock::new();

fn anchored(anchor: &Anchor, wall: impl FnOnce() -> u64) -> u64 {
    let (time, started) = anchor.get_or_init(|| (wall(), Instant::now()));
    time.saturating_add(u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX))
}

pub fn now() -> u64 {
    anchored(&START, crate::sessions::now)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn later_wall_clock_jumps_do_not_change_the_retention_clock() {
        let anchor = Anchor::new();
        let initial = anchored(&anchor, || 1234);
        let next = anchored(&anchor, || {
            panic!("a later wall clock must not be consulted")
        });
        assert!(next >= initial && next < initial + 60_000);
    }
}
