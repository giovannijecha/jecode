use std::time::{Duration, Instant};

// View state stays in the TUI. It is neither a provider argument nor session data.
#[derive(Default)]
pub struct Presentation {
    pub started: Option<Instant>,
    pub elapsed: Option<Duration>,
    pub expanded: Option<bool>,
    pub selected: bool,
}

impl Presentation {
    pub fn live() -> Self {
        Self {
            started: Some(Instant::now()),
            ..Self::default()
        }
    }
    pub fn finish(&mut self) {
        self.elapsed = self.started.map(|started| started.elapsed());
    }
    pub fn duration(&self) -> Option<Duration> {
        self.elapsed
            .or_else(|| self.started.map(|started| started.elapsed()))
    }
}
