//! Build safe display snapshots at terminal cadence, without copying each token's prefix.
use crate::redact::Redactor;
use std::time::{Duration, Instant};

const FRAME: Duration = Duration::from_millis(16);

pub(super) struct Text {
    redactor: Redactor,
    pending: Vec<u8>,
    visible: String,
    dirty: bool,
    emitted: Option<Instant>,
}

impl Text {
    pub fn new(redactor: Redactor) -> Self {
        Self {
            redactor,
            pending: Vec::new(),
            visible: String::new(),
            dirty: false,
            emitted: None,
        }
    }

    pub fn push(&mut self, delta: &str) -> Option<String> {
        self.push_at(delta, Instant::now())
    }

    fn push_at(&mut self, delta: &str, now: Instant) -> Option<String> {
        self.pending.extend_from_slice(delta.as_bytes());
        self.append_safe(false);
        if self
            .emitted
            .is_none_or(|last| now.duration_since(last) >= FRAME)
        {
            self.snapshot(now)
        } else {
            None
        }
    }

    pub fn finish(&mut self) -> Option<String> {
        self.append_safe(true);
        self.snapshot(Instant::now())
    }

    fn append_safe(&mut self, finish: bool) {
        let (consumed, safe) = self.redactor.byte_prefix(&self.pending, finish);
        if !safe.is_empty() {
            // Deltas and secrets are UTF-8; held secret prefixes start at character boundaries.
            self.visible.push_str(std::str::from_utf8(&safe).unwrap());
            self.dirty = true;
        }
        self.pending.drain(..consumed);
    }

    fn snapshot(&mut self, now: Instant) -> Option<String> {
        if !self.dirty {
            return None;
        }
        self.dirty = false;
        self.emitted = Some(now);
        Some(self.visible.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rapid_deltas_keep_the_first_and_complete_final_snapshot() {
        let mut text = Text::new(Redactor::empty());
        let now = Instant::now();
        assert_eq!(text.push_at("first", now).as_deref(), Some("first"));
        for _ in 0..2000 {
            assert!(text.push_at(" fragment", now).is_none());
        }
        assert_eq!(
            text.finish().unwrap(),
            format!("first{}", " fragment".repeat(2000))
        );
        assert!(text.finish().is_none());
    }

    #[test]
    fn cadence_emits_progress_without_waiting_for_completion() {
        let mut text = Text::new(Redactor::empty());
        let now = Instant::now();
        assert_eq!(text.push_at("one", now).as_deref(), Some("one"));
        assert_eq!(
            text.push_at(" two", now + FRAME).as_deref(),
            Some("one two")
        );
    }

    #[test]
    fn credentials_split_between_deltas_are_never_exposed() {
        let mut text = Text::new(Redactor::new("private-🙂-key".into()));
        let now = Instant::now();
        assert_eq!(text.push_at("safe private-", now).as_deref(), Some("safe "));
        assert!(text.push_at("🙂", now + FRAME).is_none());
        assert_eq!(
            text.push_at("-key end", now + FRAME * 2).as_deref(),
            Some("safe [redacted] end")
        );
        assert!(text.finish().is_none());
    }

    #[test]
    fn final_partial_secret_prefix_and_unicode_are_preserved() {
        let mut text = Text::new(Redactor::new("secret-key".into()));
        assert!(text.push("secret-").is_none());
        assert_eq!(text.finish().as_deref(), Some("secret-"));
        let mut text = Text::new(Redactor::empty());
        assert_eq!(text.push("Città 🙂").as_deref(), Some("Città 🙂"));
    }
}
