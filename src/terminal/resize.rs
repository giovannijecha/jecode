//! Coalesce geometry changes without blocking input, cancellation or model events.
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Region {
    Transcript,
    Composer,
}

#[derive(Default)]
pub struct Resize {
    observed: Option<(usize, usize)>,
    changed: Option<Instant>,
}
impl Resize {
    pub fn region(
        &mut self,
        size: (usize, usize),
        displayed: (usize, usize),
        now: Instant,
    ) -> Option<Region> {
        if self.ready(size, now) {
            Some(Region::Transcript)
        } else if size != displayed {
            // Full-width rules must follow geometry during the quiet period;
            // otherwise native reflow leaves old rules split over several rows.
            Some(Region::Composer)
        } else {
            None
        }
    }

    pub fn ready(&mut self, size: (usize, usize), now: Instant) -> bool {
        if self.observed != Some(size) {
            if self.observed.is_some() {
                self.changed = Some(now);
            }
            self.observed = Some(size);
        }
        self.changed
            .is_none_or(|changed| now.duration_since(changed) >= Duration::from_millis(100))
    }
}

#[test]
fn dragging_coalesces_and_returning_to_original_size_still_waits() {
    let now = Instant::now();
    let mut resize = Resize::default();
    assert!(resize.ready((120, 30), now));
    for n in 1..80 {
        assert!(!resize.ready((120 - n, 30), now + Duration::from_millis(n as u64 * 6)));
    }
    assert!(!resize.ready((120, 30), now + Duration::from_millis(500)));
    assert!(!resize.ready((120, 30), now + Duration::from_millis(599)));
    assert!(resize.ready((120, 30), now + Duration::from_millis(600)));
}
