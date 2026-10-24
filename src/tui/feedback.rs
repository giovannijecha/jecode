use super::state::{Kind, State};
use std::time::{Duration, Instant};

const BRIEF: Duration = Duration::from_secs(5);

#[derive(Clone, Copy)]
enum Lifetime {
    /// Results fade.
    Brief(Instant),
    /// Warnings and errors stay until the next key.
    Action,
    /// Progress runs beside the scanner until its operation ends.
    Progress(Instant),
}

/// One notice. Results and progress share it; where it appears follows its
/// lifetime: progress rides the composer rule, results a marked line, and
/// warnings and errors a banner.
pub(super) struct Feedback {
    pub kind: Kind,
    pub text: String,
    /// A neutral fact rather than a finished action: `●` instead of `✓`.
    pub info: bool,
    /// A keycap and its label, naming the next step.
    pub action: Option<(&'static str, &'static str)>,
    /// When paused progress resumes, counted down after the text.
    pub until: Option<Instant>,
    lifetime: Lifetime,
}

impl Feedback {
    pub fn result(kind: Kind, text: impl Into<String>) -> Self {
        Self::at(kind, text, Instant::now())
    }

    pub(super) fn at(kind: Kind, text: impl Into<String>, now: Instant) -> Self {
        Self {
            kind,
            text: text.into(),
            info: false,
            action: None,
            until: None,
            lifetime: if matches!(kind, Kind::Warning | Kind::Error) {
                Lifetime::Action
            } else {
                Lifetime::Brief(now + BRIEF)
            },
        }
    }

    pub fn progress(kind: Kind, text: impl Into<String>) -> Self {
        Self {
            lifetime: Lifetime::Progress(Instant::now()),
            ..Self::at(kind, text, Instant::now())
        }
    }

    pub fn info(mut self) -> Self {
        self.info = true;
        self
    }

    pub fn action(mut self, key: &'static str, label: &'static str) -> Self {
        self.action = Some((key, label));
        self
    }

    pub fn until(mut self, until: Instant) -> Self {
        self.until = Some(until);
        self
    }

    pub fn in_progress(&self) -> bool {
        matches!(self.lifetime, Lifetime::Progress(_))
    }

    /// When progress started, for its scanner frame.
    pub fn started(&self) -> Option<Instant> {
        match self.lifetime {
            Lifetime::Progress(started) => Some(started),
            _ => None,
        }
    }

    fn rank(&self) -> u8 {
        match self.kind {
            Kind::Error => 2,
            Kind::Warning => 1,
            _ => 0,
        }
    }
}

impl State {
    /// Progress takes its own slot, so it never hides a result or a banner.
    pub fn notify(&mut self, notice: Feedback) {
        if notice.in_progress() {
            self.progress = Some(notice);
        } else {
            replace(&mut self.notice, notice);
        }
    }

    pub fn notify_copy(&mut self, notice: Feedback) {
        replace(&mut self.copy_notice, notice);
    }

    pub fn clear_notice(&mut self) {
        clear(&mut self.notice);
    }

    pub fn clear_copy_notice(&mut self) {
        clear(&mut self.copy_notice);
    }

    pub fn clear_progress(&mut self) {
        self.progress = None;
    }

    /// The progress beside the scanner: the operation's own, then a copy.
    pub fn current_progress(&self) -> Option<&Feedback> {
        self.progress.as_ref().or(self
            .copy_notice
            .as_ref()
            .filter(|notice| notice.in_progress()))
    }

    pub fn feedback_input(&mut self, editing: bool) {
        for slot in [&mut self.notice, &mut self.copy_notice] {
            if slot.as_ref().is_some_and(|notice| match notice.lifetime {
                Lifetime::Brief(_) => editing,
                Lifetime::Action => true,
                Lifetime::Progress(_) => false,
            }) {
                *slot = None;
            }
        }
    }

    pub fn expire_feedback(&mut self, now: Instant) -> bool {
        let mut changed = false;
        for slot in [&mut self.notice, &mut self.copy_notice] {
            if slot.as_ref().is_some_and(
                |notice| matches!(notice.lifetime, Lifetime::Brief(until) if now >= until),
            ) {
                *slot = None;
                changed = true;
            }
        }
        changed
    }

    pub fn feedback_wait(&self, now: Instant, delay: Duration) -> Duration {
        [&self.notice, &self.copy_notice]
            .into_iter()
            .flatten()
            .filter_map(|notice| match notice.lifetime {
                Lifetime::Brief(until) => Some(until.saturating_duration_since(now)),
                _ => None,
            })
            .fold(delay, Duration::min)
    }
}

/// A banner yields only to one at least as severe.
fn replace(slot: &mut Option<Feedback>, notice: Feedback) {
    if slot
        .as_ref()
        .is_some_and(|current| !current.in_progress() && current.rank() > notice.rank())
    {
        return;
    }
    *slot = Some(notice);
}

fn clear(slot: &mut Option<Feedback>) {
    if slot
        .as_ref()
        .is_none_or(|notice| !matches!(notice.kind, Kind::Error))
    {
        *slot = None;
    }
}

#[cfg(test)]
mod tests;
