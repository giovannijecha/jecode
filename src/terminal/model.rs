use super::{Key, text::Editor};
use std::time::{Duration, Instant};

pub struct Block {
    pub speaker: &'static str,
    pub text: String,
}
pub struct Model {
    pub editor: Editor,
    pub blocks: Vec<Block>,
    pub status: &'static str,
    pub quit: bool,
    pub subtitle: String,
    pub account: Option<super::account::View>,
    pub tools: super::tool_activity::Activity,
    pub action_demo: Option<super::action_demo::Demo>,
    pub menu: super::menu::Menu,
    pub navigation: Option<super::navigation::Request>,
    tool_demo: Option<super::tool_demo::Demo>,
    pending: String,
    offset: usize,
    next: Instant,
}
impl Model {
    pub fn new(now: Instant) -> Self {
        Self {
            editor: Editor::default(),
            blocks: vec![Block {
                speaker: "Jecode",
                text: "All activity is simulated; no files or commands are used.\nTry `/code`, `/long`, `/error`, `/tools`, `/tools-error`, `/edit`, `/command` or `/command-error`.".into(),
            }],
            status: "Ready",
            quit: false,
            subtitle: "Local demo / no model connected".into(),
            account: None,
            tools: super::tool_activity::Activity::default(),
            action_demo: None,
            menu: super::menu::Menu::default(),
            navigation: None,
            tool_demo: None,
            pending: String::new(),
            offset: 0,
            next: now,
        }
    }
    pub fn streaming(&self) -> bool {
        !self.pending.is_empty()
            || self.tool_demo.is_some()
            || self.action_demo.is_some()
            || self.account.as_ref().is_some_and(|view| view.generating())
    }
    pub fn input(&mut self, key: Key, now: Instant) {
        if let Some(demo) = &mut self.action_demo
            && demo.input(&key, &mut self.blocks, now)
        {
            if demo.done() {
                self.action_demo = None;
                self.status = "Complete - local action preview";
            }
            return;
        }
        match key {
            Key::Text(text) => self.editor.insert(&text),
            Key::Left => self.editor.left(),
            Key::Right => self.editor.right(),
            Key::Home => self.editor.cursor = 0,
            Key::End => self.editor.cursor = self.editor.text.len(),
            Key::Backspace => self.editor.backspace(),
            Key::Delete => self.editor.delete(),
            Key::PageUp | Key::PageDown | Key::Tab | Key::Up | Key::Down => {}
            Key::Quit => self.quit = true,
            Key::Escape | Key::Interrupt if self.streaming() => {
                self.pending.clear();
                self.tool_demo = None;
                self.tools.close(&self.blocks, true, now);
                self.status = "Interrupted - partial output retained";
            }
            Key::Interrupt if self.editor.text.is_empty() => self.quit = true,
            Key::Interrupt => {
                self.editor.take();
            }
            Key::Enter if self.streaming() => self.status = "Streaming - draft kept; Esc stops",
            Key::Enter => self.submit(now),
            Key::Escape => {}
        }
    }
    fn submit(&mut self, now: Instant) {
        if self.editor.text.trim().is_empty() {
            return;
        }
        // Bounded preview history. Real canonical history will be a separate owner.
        let bytes: usize = self.blocks.iter().map(|b| b.text.len()).sum();
        let added_blocks = if matches!(self.editor.text.trim(), "/tools" | "/tools-error") {
            5
        } else {
            2
        };
        if self.blocks.len() + added_blocks > 65 || bytes + self.editor.text.len() + 8192 > 131_072
        {
            self.status = "Preview history full - restart the demo";
            return;
        }
        let prompt = self.editor.take();
        let response = match prompt.trim() {
            "/long" => (1..=28)
                .map(|n| format!("{n:02}. A useful harness keeps the task visible, streams progress and makes every effect explicit. Resize the window or use the terminal scrollback while this text arrives.\n\n"))
                .collect(),
            "/code" => "Here is a small Rust example (display only):\n\n```rust\nfn main() {\n    let message = \"Hello from Jecode\";\n    println!(\"{message}\");\n}\n```\n\nUnicode sample: café, 中文, 👩‍💻.\nNo compiler or command was invoked.".into(),
            "/error" => "This scenario deliberately stops after a partial response.\nThe draft remains editable; no automatic retry will occur.".into(),
            _ => "Ready. This is a **scripted response** for trying the interface.\n\n- Type `/code` to inspect code rendering.\n- Type `/long` to fill the terminal scrollback.\n- Press **Esc** during a response to stop it.\n\nYour draft stays available while text arrives.".into(),
        };
        let failure = prompt.trim() == "/error";
        self.blocks.push(Block {
            speaker: "You",
            text: prompt,
        });
        let prompt = self.blocks.last().unwrap().text.trim();
        if matches!(prompt, "/tools" | "/tools-error") {
            self.tool_demo = Some(super::tool_demo::Demo::new(now, prompt == "/tools-error"));
            self.status = "Streaming locally";
            return;
        }
        let prompt = prompt.to_owned();
        if let Some(demo) = super::action_demo::Demo::start(&prompt, &mut self.blocks, now) {
            self.action_demo = Some(demo);
            self.status = "Local action preview";
            return;
        }
        self.blocks.push(Block {
            speaker: if failure { "Demo / failure" } else { "Demo" },
            text: String::new(),
        });
        self.pending = response;
        self.offset = 0;
        self.next = now + Duration::from_millis(120);
        self.status = "Streaming locally";
    }
    pub fn tick(&mut self, now: Instant) -> bool {
        if let Some(demo) = &mut self.action_demo {
            let changed = demo.tick(&mut self.blocks, now, self.tools.reduced_motion);
            if demo.done() {
                self.action_demo = None;
                self.status = "Complete - local action preview";
            }
            return changed;
        }
        let command_animated = self
            .account
            .as_mut()
            .and_then(|v| v.command.as_mut())
            .is_some_and(|run| run.spinner.tick(now, self.tools.reduced_motion));
        let animated = self.tools.tick(now) || command_animated;
        if let Some(mut demo) = self.tool_demo.take() {
            let changed = demo.tick(self, now);
            if !demo.done() {
                self.tool_demo = Some(demo);
            }
            return animated || changed;
        }
        if self.pending.is_empty() || now < self.next {
            return animated;
        }
        let end = self.pending[self.offset..]
            .char_indices()
            .nth(8)
            .map_or(self.pending.len(), |(n, _)| self.offset + n);
        self.blocks
            .last_mut()
            .unwrap()
            .text
            .push_str(&self.pending[self.offset..end]);
        self.offset = end;
        self.next = now + Duration::from_millis(30);
        if end == self.pending.len() {
            self.pending.clear();
            self.status = if self.blocks.last().unwrap().speaker == "Demo / failure" {
                "Simulated stream failure - partial output retained"
            } else {
                "Complete - local demo"
            };
        }
        true
    }
    pub fn start_tool(&mut self, name: &'static str, path: String, now: Instant) -> usize {
        let index = self.blocks.len();
        self.blocks.push(Block {
            speaker: "Tool",
            text: if path.is_empty() {
                name.into()
            } else {
                format!("{name} / {path}")
            },
        });
        self.tools.begin(index, name, &self.blocks, now);
        index
    }
    pub fn finish_tool(
        &mut self,
        index: usize,
        summary: &str,
        failed: bool,
        limited: bool,
        now: Instant,
    ) {
        if let Some(block) = self.blocks.get_mut(index) {
            block.text.push_str(&format!(
                "\n  {}{summary}",
                if failed { "failed / " } else { "" }
            ));
            self.tools.finish(failed, limited, now);
        }
    }
}
