use std::time::{Duration, Instant};

// The quiet period starts at the latest observed
// geometry, even when a drag returns to the dimensions already rendered.
const SETTLE: Duration = Duration::from_millis(75);

pub(super) struct Resize {
    size: (usize, usize),
    deadline: Option<Instant>,
}

impl Resize {
    pub fn new(size: (usize, usize)) -> Self {
        Self {
            size,
            deadline: None,
        }
    }
    pub fn observe(&mut self, size: (usize, usize), now: Instant) {
        if self.size != size {
            self.size = size;
            self.deadline = Some(now + SETTLE);
        }
    }
    pub fn pending(&self) -> bool {
        self.deadline.is_some()
    }
    pub fn due(&self, now: Instant) -> bool {
        self.deadline.is_some_and(|deadline| now >= deadline)
    }
    pub fn take_due(&mut self, now: Instant) -> bool {
        if self.due(now) {
            self.deadline = None;
            true
        } else {
            false
        }
    }
    pub fn wait(&self, now: Instant) -> Duration {
        self.deadline.map_or(Duration::from_millis(30), |deadline| {
            deadline
                .saturating_duration_since(now)
                .min(Duration::from_millis(30))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_latest_change_restarts_the_quiet_period_even_after_returning_to_the_original_size() {
        let start = Instant::now();
        let mut resize = Resize::new((80, 24));
        resize.observe((40, 24), start);
        resize.observe((80, 24), start + Duration::from_millis(70));
        assert!(!resize.due(start + Duration::from_millis(100)));
        assert!(resize.take_due(start + Duration::from_millis(145)));
        assert!(!resize.pending());
    }

    #[test]
    fn repeated_samples_do_not_postpone_reconstruction_and_height_changes_do() {
        let start = Instant::now();
        let mut resize = Resize::new((80, 24));
        resize.observe((80, 10), start);
        resize.observe((80, 10), start + Duration::from_millis(60));
        assert_eq!(
            resize.wait(start + Duration::from_millis(70)),
            Duration::from_millis(5)
        );
        assert!(resize.take_due(start + SETTLE));
        assert!(!resize.take_due(start + Duration::from_secs(1)));
    }
}
