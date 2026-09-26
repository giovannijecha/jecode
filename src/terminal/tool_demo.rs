//! Inert tool events for the local preview. No filesystem, controller or provider.
use super::model::{Block, Model};
use std::time::{Duration, Instant};

pub struct Demo {
    step: u8,
    next: Instant,
    call: usize,
    failure: bool,
}
impl Demo {
    pub fn new(now: Instant, failure: bool) -> Self {
        Self {
            step: 0,
            next: now,
            call: 0,
            failure,
        }
    }
    pub fn done(&self) -> bool {
        self.step == 8
    }
    pub fn tick(&mut self, model: &mut Model, now: Instant) -> bool {
        if now < self.next {
            return false;
        }
        match self.step {
            0 => self.call = model.start_tool("list_files", "src".into(), now),
            1 => model.finish_tool_output(self.call, "8 entries / 0 omitted", false, false,
                "account.rs\neditor.rs\nmodel.rs\nrender.rs\nview.rs\ninput.rs\nmenu.rs\nrecovery.rs\n", now),
            2 => self.call = model.start_tool("read_file", "src/session/mod.rs".into(), now),
            3 => model.finish_tool_output(self.call, "80 lines", false, false,
                &(1..=12).map(|line| format!("line {line}\n")).collect::<String>(), now),
            4 => model.tools.waiting("Waiting for model"),
            5 => self.call = model.start_tool("search_text", "src/terminal".into(), now),
            6 => model.finish_tool_output(
                self.call,
                if self.failure {
                    "permission denied (simulated)"
                } else {
                    "4 matching lines / 6 files / 0 omitted"
                },
                self.failure,
                false,
                if self.failure { "permission denied (simulated)" }
                    else { "src/terminal/view.rs:15: frame\nsrc/terminal/model.rs:80: input" },
                now,
            ),
            _ => {
                model.tools.close(&model.blocks, false, now);
                model.blocks.push(Block {
                    speaker: "Demo",
                    text: "This was a **local tool preview**. No files were read, no commands ran and no model was contacted.\n\nThe completed activity stays in scrollback; your draft stays editable.".into(),
                });
                model.status = "Complete - local tool demo";
            }
        }
        self.step += 1;
        self.next = now + Duration::from_millis(650);
        true
    }
}
