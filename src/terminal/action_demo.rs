//! Inert direct edit/command presentations with no filesystem or process effects.
use super::{Key, model::Block, spinner::Spinner};
use std::time::{Duration, Instant};

#[derive(Clone, Copy, PartialEq)]
pub enum Kind {
    Edit,
    Command,
    CommandError,
}
#[derive(PartialEq)]
enum Phase {
    Running,
    Done,
}
pub struct Demo {
    pub kind: Kind,
    pub spinner: Spinner,
    phase: Phase,
    block: usize,
    started: Instant,
    observed: Instant,
    next: Instant,
    step: usize,
}
impl Demo {
    pub fn start(prompt: &str, blocks: &mut Vec<Block>, now: Instant) -> Option<Self> {
        let kind = match prompt {
            "/edit" => Kind::Edit,
            "/command" => Kind::Command,
            "/command-error" => Kind::CommandError,
            _ => return None,
        };
        let block = blocks.len();
        blocks.push(if kind == Kind::Edit {
            Block {
                speaker: "EditPreview",
                text: "  Edit src/settings.rs · +1 -1\n  @@ retry policy\n    pub fn retry_limit() -> usize {\n-       2\n+       3\n    }".into(),
            }
        } else {
            Block {
                speaker: "CommandPreview",
                text: "  Run command\n  $ cargo test --lib\n  cwd: demo-project".into(),
            }
        });
        let mut spinner = Spinner::default();
        spinner.reset(now);
        Some(Self {
            kind,
            spinner,
            phase: Phase::Running,
            block,
            started: now,
            observed: now,
            next: now + Duration::from_millis(650),
            step: 0,
        })
    }
    pub fn done(&self) -> bool {
        self.phase == Phase::Done
    }
    pub fn elapsed(&self) -> Duration {
        self.observed.saturating_duration_since(self.started)
    }
    /// During the simulated run normal editor keys remain available to the draft.
    pub fn input(&mut self, key: &Key, blocks: &mut [Block], _now: Instant) -> bool {
        if matches!(key, Key::Quit) {
            return false;
        }
        if matches!(key, Key::Escape | Key::Interrupt) {
            self.end(blocks, "! Preview interrupted · partial output retained");
            return true;
        }
        false
    }
    pub fn tick(&mut self, blocks: &mut [Block], now: Instant, reduced_motion: bool) -> bool {
        if self.phase != Phase::Running {
            return false;
        }
        let animated = self.spinner.tick(now, reduced_motion);
        self.observed = now;
        if now < self.next {
            return animated;
        }
        if self.kind == Kind::Edit {
            self.end(
                blocks,
                &format!(
                    "✓ Simulated edit complete · +1 -1 · {:.1}s",
                    self.elapsed().as_secs_f64()
                ),
            );
        } else if self.step < 3 {
            let line = match (self.step, self.kind) {
                (0, _) => "running 2 tests",
                (1, _) => "test settings::keeps_defaults ... ok",
                (_, Kind::CommandError) => "test settings::retry_limit ... FAILED",
                _ => "test settings::retry_limit ... ok",
            };
            blocks[self.block].text.push_str(&format!("\n  {line}"));
            self.step += 1;
        } else {
            let (marker, result) = if self.kind == Kind::CommandError {
                blocks[self.block]
                    .text
                    .push_str("\n  expected: 3, received: 2");
                ("!", "exit 101 · 1 passed, 1 failed")
            } else {
                ("✓", "exit 0 · 2 passed")
            };
            self.end(
                blocks,
                &format!(
                    "{marker} Simulated command complete · {result} · {:.1}s",
                    self.elapsed().as_secs_f64()
                ),
            );
        }
        self.next = now + Duration::from_millis(650);
        true
    }
    fn end(&mut self, blocks: &mut [Block], result: &str) {
        blocks[self.block].text.push('\n');
        blocks[self.block].text.push_str(result);
        self.phase = Phase::Done;
    }
}
