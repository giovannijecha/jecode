//! A presentation-only clock shared by active tool surfaces.
use std::time::{Duration, Instant};

const FRAMES: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

#[derive(Default)]
pub struct Spinner {
    frame: usize,
    last_tick: Option<Instant>,
}
impl Spinner {
    pub fn reset(&mut self, now: Instant) {
        self.frame = 0;
        self.last_tick = Some(now);
    }
    pub fn tick(&mut self, now: Instant, reduced_motion: bool) -> bool {
        let interval = if reduced_motion { 1000 } else { 80 };
        if self.last_tick.is_some_and(|last| {
            now.saturating_duration_since(last) < Duration::from_millis(interval)
        }) {
            return false;
        }
        self.last_tick = Some(now);
        if !reduced_motion {
            self.frame = (self.frame + 1) % FRAMES.len();
        }
        true
    }
    #[cfg(test)]
    pub fn marker(&self, reduced_motion: bool) -> &'static str {
        if reduced_motion {
            "⠿"
        } else {
            FRAMES[self.frame]
        }
    }
}
