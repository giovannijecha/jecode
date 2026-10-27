use super::state::{Kind, State};
use std::time::{Duration, Instant};

const BRIEF: Duration = Duration::from_secs(5);

#[derive(Clone, Copy)]
enum Lifetime {
    /// Results fade.
    Brief(Instant),
    /// Warnings stay until the next key.
    Action,
    /// Errors and turn endings stay until the next send.
    Send,
    /// Progress runs beside the caret until its operation ends.
    Progress,
}

/// How a turn ended, as the status row keeps it until the next send.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Ending {
    Done,
    Stopped,
    Failed,
}

/// One notice in the status row: `Subject · fact · fact`, its lifetime and
/// an optional keycap naming the next step.
pub(super) struct Feedback {
    pub kind: Kind,
    pub text: String,
    /// Whether this result describes a setting changing from an old value to a new one.
    pub change: bool,
    /// A keycap and its label, naming the next step.
    pub action: Option<(&'static str, &'static str)>,
    /// When paused progress resumes, counted down after the text.
    pub until: Option<Instant>,
    /// The turn ending this notice reports, if any.
    pub ending: Option<Ending>,
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
            change: false,
            action: None,
            until: None,
            ending: None,
            lifetime: match kind {
                Kind::Error => Lifetime::Send,
                Kind::Warning => Lifetime::Action,
                _ => Lifetime::Brief(now + BRIEF),
            },
        }
    }

    /// `Done`, `Stopped` or `Failed` with its facts, kept until the next send.
    pub fn ending(ending: Ending, facts: impl Into<String>) -> Self {
        let (kind, subject) = match ending {
            Ending::Done => (Kind::Notice, "Done"),
            Ending::Stopped => (Kind::Warning, "Stopped"),
            Ending::Failed => (Kind::Error, "Failed"),
        };
        let facts = facts.into();
        let text = if facts.is_empty() {
            subject.into()
        } else {
            format!("{subject} · {facts}")
        };
        Self {
            ending: Some(ending),
            lifetime: Lifetime::Send,
            ..Self::at(kind, text, Instant::now())
        }
    }

    pub fn progress(kind: Kind, text: impl Into<String>) -> Self {
        Self {
            lifetime: Lifetime::Progress,
            ..Self::at(kind, text, Instant::now())
        }
    }

    pub fn action(mut self, key: &'static str, label: &'static str) -> Self {
        self.action = Some((key, label));
        self
    }

    pub fn change(mut self) -> Self {
        self.change = true;
        self
    }

    pub fn until(mut self, until: Instant) -> Self {
        self.until = Some(until);
        self
    }

    pub fn in_progress(&self) -> bool {
        matches!(self.lifetime, Lifetime::Progress)
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
    /// Progress and turn endings take their own slots, so neither hides a
    /// result, warning or error.
    pub fn notify(&mut self, notice: Feedback) {
        if notice.in_progress() {
            self.progress = Some(notice);
        } else if notice.ending.is_some() {
            self.ending = Some(notice);
        } else {
            replace(&mut self.notice, notice);
        }
    }

    /// A send clears what waited for it: errors and the last turn's ending.
    pub fn sent(&mut self) {
        self.ending = None;
        for slot in [&mut self.notice, &mut self.copy_notice] {
            if slot.as_ref().is_some_and(|notice| !notice.in_progress()) {
                *slot = None;
            }
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
        self.recovery = None;
    }

    /// Retry countdowns overlay the operation's progress; a copy is last.
    pub fn current_progress(&self) -> Option<&Feedback> {
        self.recovery.as_ref().or(self.progress.as_ref()).or(self
            .copy_notice
            .as_ref()
            .filter(|notice| notice.in_progress()))
    }

    pub fn feedback_input(&mut self, editing: bool) {
        for slot in [&mut self.notice, &mut self.copy_notice] {
            if slot.as_ref().is_some_and(|notice| match notice.lifetime {
                Lifetime::Brief(_) => editing,
                Lifetime::Action => true,
                Lifetime::Send | Lifetime::Progress => false,
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
