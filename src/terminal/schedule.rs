//! Coalesce paints, not source events. New requests never postpone a due frame.
use std::time::{Duration, Instant};

const INTERVAL: Duration = Duration::from_millis(16);

#[derive(Default)]
pub struct PaintSchedule {
    pending: bool,
    last: Option<Instant>,
}
impl PaintSchedule {
    pub fn request(&mut self) {
        self.pending = true;
    }
    pub fn ready(&self, now: Instant) -> bool {
        self.pending
            && self
                .last
                .is_none_or(|last| now.duration_since(last) >= INTERVAL)
    }
    pub fn painted(&mut self, now: Instant) {
        self.pending = false;
        self.last = Some(now);
    }
}

#[test]
fn bursts_coalesce_without_starvation_or_idle_painting() {
    let now = Instant::now();
    let mut paint = PaintSchedule::default();
    assert!(!paint.ready(now));
    paint.request();
    assert!(paint.ready(now));
    paint.painted(now);
    for n in 1..16 {
        paint.request();
        assert!(!paint.ready(now + Duration::from_millis(n)));
    }
    assert!(paint.ready(now + INTERVAL));
    paint.painted(now + INTERVAL);
    assert!(!paint.ready(now + Duration::from_secs(1)));
    paint.request();
    // A late frame does not cause a burst of catch-up paints.
    paint.painted(now + Duration::from_secs(1));
    paint.request();
    assert!(!paint.ready(now + Duration::from_millis(1001)));
    assert!(paint.ready(now + Duration::from_millis(1016)));
}
