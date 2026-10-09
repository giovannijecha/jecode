use super::state::{Kind, State};
use std::time::{Duration, Instant};

const BRIEF: Duration = Duration::from_secs(5);

#[derive(Clone, Copy)]
enum Lifetime {
    Brief(Instant),
    Action,
    Progress,
}

pub(super) struct Feedback {
    pub kind: Kind,
    pub text: String,
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
            lifetime: if matches!(kind, Kind::Error) {
                Lifetime::Action
            } else {
                Lifetime::Brief(now + BRIEF)
            },
        }
    }

    pub fn progress(kind: Kind, text: impl Into<String>) -> Self {
        Self {
            kind,
            text: text.into(),
            lifetime: Lifetime::Progress,
        }
    }

    pub fn in_progress(&self) -> bool {
        matches!(self.lifetime, Lifetime::Progress)
    }
}

impl State {
    pub fn notify(&mut self, notice: Feedback) {
        replace(&mut self.notice, notice);
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
        if self.notice.as_ref().is_some_and(Feedback::in_progress) {
            self.notice = None;
        }
    }

    pub fn feedback_input(&mut self, editing: bool) {
        for slot in [&mut self.notice, &mut self.copy_notice] {
            if slot.as_ref().is_some_and(|notice| match notice.lifetime {
                Lifetime::Brief(_) => editing,
                Lifetime::Action => true,
                Lifetime::Progress => false,
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

fn replace(slot: &mut Option<Feedback>, notice: Feedback) {
    if !matches!(notice.kind, Kind::Error)
        && slot
            .as_ref()
            .is_some_and(|current| matches!(current.kind, Kind::Error))
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
