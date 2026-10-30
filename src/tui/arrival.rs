//! A question or permission request that arrives while the user is typing a
//! chat draft must not turn the rest of that typing into an answer. Until the
//! keys pause, or an arrow or Tab moves deliberately, typed text continues the
//! hidden draft, Enter does nothing and no digit chooses.

use std::time::{Duration, Instant};

/// A pause this long ends the typing a request arrived into.
pub const QUIET: Duration = Duration::from_millis(800);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Guard {
    last: Instant,
}

impl Guard {
    /// Guards an arrival when the draft was typed in within `QUIET`.
    pub fn arrive(typed: Option<Instant>, now: Instant) -> Option<Self> {
        typed
            .filter(|&at| now.saturating_duration_since(at) < QUIET)
            .map(|last| Self { last })
    }

    /// Whether a key at `now` still belongs to the interrupted typing; each
    /// such key extends it.
    pub fn holds(&mut self, now: Instant) -> bool {
        if now.saturating_duration_since(self.last) >= QUIET {
            return false;
        }
        self.last = now;
        true
    }

    #[cfg(test)]
    pub fn quiet(&mut self) {
        self.last = Instant::now().checked_sub(QUIET).unwrap_or(self.last);
    }
}

/// Arrows and Tab are deliberate moves within a request.
pub fn navigation(code: u16) -> bool {
    matches!(code, 9 | 37..=40)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typing_guards_an_arrival_until_the_keys_pause() {
        let now = Instant::now();
        assert!(Guard::arrive(None, now).is_none());
        let mut guard = Guard::arrive(Some(now), now + QUIET / 2).unwrap();
        assert!(guard.holds(now + QUIET * 3 / 4));
        assert!(guard.holds(now + QUIET * 3 / 2));
        assert!(!guard.holds(now + QUIET * 3));
        assert!(Guard::arrive(Some(now), now + QUIET).is_none());
        assert!(navigation(9) && navigation(38) && !navigation(13));
    }
}
