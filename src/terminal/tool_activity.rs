//! Presentation receipts for consecutive reads. Raw tool blocks remain intact.
use super::{model::Block, spinner::Spinner};
use std::{
    ops::Range,
    time::{Duration, Instant},
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Outcome {
    Running,
    Complete,
    Limited,
    Failed,
    Interrupted,
}

pub struct Call {
    pub block: usize,
    pub name: &'static str,
    pub outcome: Outcome,
}
pub struct Group {
    pub range: Range<usize>,
    pub calls: Vec<Call>,
    pub summary: Option<Block>,
    pub state: &'static str,
    started: Instant,
    observed: Instant,
}
#[derive(Default)]
pub struct Activity {
    pub groups: Vec<Group>,
    pub reduced_motion: bool,
    spinner: Spinner,
}
impl Activity {
    pub fn active(&self) -> Option<&Group> {
        self.groups.last().filter(|g| g.summary.is_none())
    }
    fn active_mut(&mut self) -> Option<&mut Group> {
        self.groups.last_mut().filter(|g| g.summary.is_none())
    }
    pub fn begin(&mut self, index: usize, name: &'static str, blocks: &[Block], now: Instant) {
        // Effects and unknown tools must retain their individual presentation.
        if !matches!(name, "read_file" | "list_files" | "search_text") {
            self.close(blocks, false, now);
            return;
        }
        if self.active().is_none() {
            self.groups.push(Group {
                range: index..index + 1,
                calls: Vec::new(),
                summary: None,
                state: "Reading workspace",
                started: now,
                observed: now,
            });
            self.spinner.reset(now);
        }
        let group = self.active_mut().unwrap();
        group.range.end = index + 1;
        group.observed = now;
        group.state = "Reading workspace";
        group.calls.push(Call {
            block: index,
            name,
            outcome: Outcome::Running,
        });
    }
    pub fn finish(&mut self, failed: bool, limited: bool, now: Instant) {
        if let Some(group) = self.active_mut() {
            if let Some(call) = group.calls.last_mut() {
                call.outcome = if failed {
                    Outcome::Failed
                } else if limited {
                    Outcome::Limited
                } else {
                    Outcome::Complete
                };
            }
            group.observed = now;
            group.state = "Processing results";
        }
    }
    pub fn waiting(&mut self, state: &'static str) {
        if let Some(group) = self.active_mut() {
            group.state = state;
        }
    }
    pub fn close(&mut self, blocks: &[Block], interrupted: bool, now: Instant) {
        let Some(group) = self.active_mut() else {
            return;
        };
        group.observed = now;
        for call in &mut group.calls {
            if call.outcome == Outcome::Running {
                call.outcome = Outcome::Interrupted;
            }
        }
        let incomplete = interrupted
            || group
                .calls
                .iter()
                .any(|c| c.outcome == Outcome::Interrupted);
        let failed = group.calls.iter().any(|c| c.outcome == Outcome::Failed);
        let title = if incomplete {
            "Exploration interrupted"
        } else if failed {
            "Exploration finished with errors"
        } else {
            "Explored workspace"
        };
        let marker = if incomplete || failed { "!" } else { "✓" };
        let mut text = format!(
            "{marker} {title} · {} · {:.1}s",
            group.counts(),
            group.elapsed().as_secs_f64()
        );
        // Exceptional results remain visible, including truncation and omissions.
        // Ordinary receipts are retained in the original blocks, not discarded.
        for call in &group.calls {
            if matches!(
                call.outcome,
                Outcome::Limited | Outcome::Failed | Outcome::Interrupted
            ) {
                let block = &blocks[call.block];
                let label = match call.outcome {
                    Outcome::Limited => "limited",
                    Outcome::Failed => "failed",
                    _ => "no result received",
                };
                text.push_str(&format!(
                    "\n  {label} / {}",
                    block.text.replace('\n', " / ")
                ));
            }
        }
        group.summary = Some(Block {
            speaker: if failed || incomplete {
                "ToolWarning"
            } else {
                "ToolSummary"
            },
            text,
        });
    }
    /// Only an active group advances the animation; idle/finalized views stay still.
    pub fn tick(&mut self, now: Instant) -> bool {
        if self.active().is_none() {
            return false;
        }
        if !self.spinner.tick(now, self.reduced_motion) {
            return false;
        }
        self.active_mut().unwrap().observed = now;
        true
    }
    #[cfg(test)]
    pub fn marker(&self) -> &'static str {
        self.spinner.marker(self.reduced_motion)
    }
}
impl Group {
    pub fn elapsed(&self) -> Duration {
        self.observed.saturating_duration_since(self.started)
    }
    pub fn counts(&self) -> String {
        let mut parts = Vec::new();
        // Put exceptions first so a narrow active row cannot hide them behind counts.
        for (outcome, label) in [(Outcome::Failed, "failed"), (Outcome::Limited, "limited")] {
            let count = self.calls.iter().filter(|c| c.outcome == outcome).count();
            if count > 0 {
                parts.push(format!("{count} {label}"));
            }
        }
        for (name, singular, plural) in [
            ("read_file", "read", "reads"),
            ("search_text", "search", "searches"),
            ("list_files", "listing", "listings"),
        ] {
            let count = self
                .calls
                .iter()
                .filter(|c| {
                    c.name == name && matches!(c.outcome, Outcome::Complete | Outcome::Limited)
                })
                .count();
            if count > 0 {
                parts.push(format!(
                    "{count} {}",
                    if count == 1 { singular } else { plural }
                ));
            }
        }
        if parts.is_empty() {
            "No completed reads yet".into()
        } else {
            parts.join(" · ")
        }
    }
}
