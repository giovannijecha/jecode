use super::{Key, editor::Editor, spinner::Spinner};
use std::collections::{BTreeMap, VecDeque};
use std::time::{Duration, Instant};

#[derive(Clone)]
pub struct Block {
    pub speaker: &'static str,
    pub text: String,
}
pub struct Model {
    pub editor: Editor,
    pub blocks: Vec<Block>,
    pub status: &'static str,
    pub edit_notice: &'static str,
    pub quit: bool,
    pub account: Option<super::account::View>,
    pub tools: super::tool_activity::Activity,
    pub tool_details: BTreeMap<usize, super::lab::model::Tool>,
    pub command_receipts: BTreeMap<usize, super::lab::model::Receipt>,
    tool_started: BTreeMap<usize, Instant>,
    pub expanded: bool,
    pub status_spinner: Spinner,
    pub action_demo: Option<super::action_demo::Demo>,
    pub menu: super::menu::Menu,
    pub prompt_history: super::prompt_history::PromptHistory,
    pub navigation: Option<super::navigation::Request>,
    tool_demo: Option<super::tool_demo::Demo>,
    pub demo_queue: VecDeque<String>,
    pending: String,
    offset: usize,
    next: Instant,
}
impl Model {
    pub fn new(now: Instant) -> Self {
        let mut status_spinner = Spinner::default();
        status_spinner.reset(now);
        Self {
            editor: Editor::default(),
            blocks: vec![Block {
                speaker: "Jecode",
                text: "All activity is simulated; no files or commands are used.\nTry `/code`, `/long`, `/error`, `/tools`, `/tools-error`, `/edit`, `/command` or `/command-error`.".into(),
            }],
            status: "Ready",
            edit_notice: "",
            quit: false,
            account: None,
            tools: super::tool_activity::Activity::default(),
            tool_details: BTreeMap::new(),
            command_receipts: BTreeMap::new(),
            tool_started: BTreeMap::new(),
            expanded: false,
            status_spinner,
            action_demo: None,
            menu: super::menu::Menu::default(),
            prompt_history: super::prompt_history::PromptHistory::default(),
            navigation: None,
            tool_demo: None,
            demo_queue: VecDeque::new(),
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
            self.tool_details.insert(
                demo.block_index(),
                demo.tool(&self.blocks[demo.block_index()]),
            );
            if demo.done() {
                self.action_demo = None;
                self.status = "Complete - local action preview";
                self.recover_demo_queue();
            }
            return;
        }
        if matches!(
            key,
            Key::Text(_)
                | Key::Paste(_)
                | Key::Newline
                | Key::Backspace
                | Key::Delete
                | Key::WordBackspace
                | Key::WordDelete
                | Key::LineBackspace
        ) && self.prompt_history.browsing()
        {
            self.prompt_history.edited();
            self.menu.pasted_literal = false;
        }
        match key {
            Key::Text(text) => self.insert(&text),
            Key::Paste(text) => {
                let literal = self.editor.cursor == 0 && text.starts_with('/');
                if self.editor.insert(&text) {
                    self.menu.pasted_literal |= literal;
                    self.edit_notice = "";
                } else {
                    self.edit_error("Input exceeds 256 KiB / draft kept");
                }
            }
            Key::PasteRejected(message) => self.edit_error(message),
            Key::Newline => self.insert("\n"),
            Key::LineBackspace => self.editor.line_backspace(),
            Key::Expand => self.expanded = !self.expanded,
            Key::Left => self.editor.left(),
            Key::Right => self.editor.right(),
            Key::WordLeft => self.editor.word_left(),
            Key::WordRight => self.editor.word_right(),
            Key::Home => self.editor.home(),
            Key::End => self.editor.end(),
            Key::DraftStart => self.editor.draft_start(),
            Key::DraftEnd => self.editor.draft_end(),
            Key::Backspace => self.editor.backspace(),
            Key::Delete => self.editor.delete(),
            Key::WordBackspace => self.editor.word_backspace(),
            Key::WordDelete => self.editor.word_delete(),
            Key::Up => {
                if !self.editor.vertical(false) {
                    self.prompt_history
                        .previous(&mut self.editor, &mut self.menu.pasted_literal);
                }
            }
            Key::Down => {
                if !self.editor.vertical(true) {
                    self.prompt_history
                        .next(&mut self.editor, &mut self.menu.pasted_literal);
                }
            }
            Key::PageUp | Key::HistoryPrevious => self
                .prompt_history
                .previous(&mut self.editor, &mut self.menu.pasted_literal),
            Key::PageDown | Key::HistoryNext => self
                .prompt_history
                .next(&mut self.editor, &mut self.menu.pasted_literal),
            Key::Tab | Key::RetrieveQueued | Key::AbandonRecovered => {}
            Key::Quit => self.quit = true,
            Key::Escape | Key::Interrupt if self.streaming() => {
                self.pending.clear();
                self.tool_demo = None;
                self.tools.close(&self.blocks, true, now);
                self.close_running_tools("preview interrupted; outcome incomplete");
                self.status = "Interrupted - partial output retained";
                self.recover_demo_queue();
            }
            Key::Interrupt if self.editor.text.is_empty() => self.quit = true,
            Key::Interrupt => {
                self.editor.take();
                self.menu.pasted_literal = false;
                self.edit_notice = "";
            }
            Key::Enter if self.streaming() => {
                if !self.editor.text.trim().is_empty() {
                    if self.demo_queue.len() < 8
                        && self.editor.text.len() <= crate::session::MAX_PROMPT_BYTES
                    {
                        self.demo_queue.push_back(self.editor.take());
                    } else {
                        self.edit_error("Queue full or prompt exceeds 256 KiB / draft kept");
                    }
                }
            }
            Key::Enter => self.submit(now),
            Key::Escape => {}
        }
    }
    fn insert(&mut self, text: &str) {
        if !self.editor.insert(text) {
            self.edit_error("Input exceeds 256 KiB / draft kept");
        } else {
            self.edit_notice = "";
        }
    }
    fn edit_error(&mut self, message: &'static str) {
        if let Some(view) = &mut self.account {
            view.local_notice = message.into();
            view.local_failed = true;
        } else {
            self.edit_notice = message;
        }
    }
    fn recover_demo_queue(&mut self) {
        if self.demo_queue.is_empty() {
            return;
        }
        let mut drafts: Vec<String> = self.demo_queue.drain(..).collect();
        if !self.editor.text.is_empty() {
            drafts.push(self.editor.take());
        }
        self.editor.replace(&drafts.join("\n"));
        self.menu.pasted_literal = true;
    }
    fn dispatch_demo_queue(&mut self, now: Instant) {
        let Some(next) = self.demo_queue.pop_front() else {
            return;
        };
        let draft = self.editor.take();
        self.editor.replace(&next);
        self.submit(now);
        if self.streaming() {
            self.editor.replace(&draft);
        } else {
            self.demo_queue.push_front(next);
            self.editor.replace(&draft);
            self.recover_demo_queue();
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
        self.edit_notice = "";
        self.prompt_history.record(&prompt);
        self.menu.pasted_literal = false;
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
            self.tool_details.insert(
                demo.block_index(),
                demo.tool(&self.blocks[demo.block_index()]),
            );
            self.action_demo = Some(demo);
            self.status = "Local action preview";
            return;
        }
        self.blocks.push(Block {
            speaker: if failure { "Demo / failure" } else { "Demo" },
            text: String::new(),
        });
        self.pending = response;
        self.status_spinner.reset(now);
        self.offset = 0;
        self.next = now + Duration::from_millis(120);
        self.status = "Streaming locally";
    }
    pub fn tick(&mut self, now: Instant) -> bool {
        if let Some(demo) = &mut self.action_demo {
            let changed = demo.tick(&mut self.blocks, now, self.tools.reduced_motion);
            self.tool_details.insert(
                demo.block_index(),
                demo.tool(&self.blocks[demo.block_index()]),
            );
            if demo.done() {
                self.action_demo = None;
                self.status = "Complete - local action preview";
                self.dispatch_demo_queue(now);
            }
            return changed;
        }
        let command_animated = self
            .account
            .as_mut()
            .and_then(|v| v.command.as_mut())
            .is_some_and(|run| run.spinner.tick(now, self.tools.reduced_motion));
        let tool_animated = self.tools.tick(now);
        let status_animated = super::activity_view::model_label(self).is_some()
            && !self.tools.reduced_motion
            && self.status_spinner.tick(now, false);
        let animated = tool_animated || command_animated || status_animated;
        if let Some(mut demo) = self.tool_demo.take() {
            let changed = demo.tick(self, now);
            if !demo.done() {
                self.tool_demo = Some(demo);
            } else {
                self.dispatch_demo_queue(now);
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
            if self.blocks.last().unwrap().speaker != "Demo / failure" {
                self.dispatch_demo_queue(now);
            } else {
                self.recover_demo_queue();
            }
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
        let verb = Self::tool_verb(name);
        self.tool_details.insert(
            index,
            super::lab::model::Tool {
                verb: verb.into(),
                subject: path,
                summary: String::new(),
                status: super::lab::model::Status::Running,
                elapsed_ms: 0,
                detail: super::lab::model::Detail::Output(String::new()),
            },
        );
        self.tool_started.insert(index, now);
        index
    }
    fn tool_verb(name: &str) -> &str {
        match name {
            "read_file" => "Read",
            "search_text" => "Search",
            "list_files" => "List",
            "view_image" => "View image",
            "index_receipts" => "Find receipts",
            "recall_receipts" => "Recall receipts",
            "run_command" => "Run",
            "create_file" => "Create",
            "edit_file" => "Edit",
            _ => name,
        }
    }
    pub fn restored_tool(source: crate::session::TranscriptTool) -> super::lab::model::Tool {
        super::lab::model::Tool {
            verb: Self::tool_verb(&source.name).into(),
            subject: source.subject,
            summary: if source.outcome_unknown {
                format!("{} · saved outcome unavailable", source.summary)
            } else {
                source.summary
            },
            status: if source.failed {
                super::lab::model::Status::Failed
            } else if source.limited || source.outcome_unknown {
                super::lab::model::Status::Warned
            } else {
                super::lab::model::Status::Done
            },
            // Canonical receipts do not store a duration. Do not display 0.0s.
            elapsed_ms: u64::MAX,
            detail: super::lab::model::Detail::Output(Self::tool_output(&source.output)),
        }
    }
    pub fn close_running_tools(&mut self, reason: &str) {
        for tool in self.tool_details.values_mut() {
            if tool.status == super::lab::model::Status::Running {
                tool.status = super::lab::model::Status::Warned;
                tool.summary = reason.into();
            }
        }
    }
    fn tool_output(raw: &str) -> String {
        let Ok(value) = crate::json::parse(
            raw,
            crate::json::Limits {
                bytes: raw.len().max(1),
                nodes: 100_000,
                depth: 64,
            },
        ) else {
            return raw.into();
        };
        if let Some(text) = value.get("text").and_then(crate::json::Value::text) {
            return text.into();
        }
        if let Some(entries) = value.get("entries").and_then(crate::json::Value::array) {
            return entries
                .iter()
                .filter_map(|entry| {
                    let name = entry.get("name")?.text()?;
                    let suffix = if entry.get("type").and_then(crate::json::Value::text)
                        == Some("directory")
                    {
                        "/"
                    } else {
                        ""
                    };
                    Some(format!("{name}{suffix}"))
                })
                .collect::<Vec<_>>()
                .join("\n");
        }
        if let Some(found) = value.get("matches").and_then(crate::json::Value::array) {
            return found
                .iter()
                .filter_map(|entry| {
                    Some(format!(
                        "{}:{}: {}",
                        entry.get("path")?.text()?,
                        entry.get("line")?.unsigned()?,
                        entry.get("text")?.text()?
                    ))
                })
                .collect::<Vec<_>>()
                .join("\n");
        }
        if let Some(error) = value.get("error").and_then(crate::json::Value::text) {
            return error.into();
        }
        raw.into()
    }
    #[cfg(test)]
    pub fn finish_tool(
        &mut self,
        index: usize,
        summary: &str,
        failed: bool,
        limited: bool,
        now: Instant,
    ) {
        self.finish_tool_output(index, summary, failed, limited, "", now);
    }
    pub fn finish_tool_output(
        &mut self,
        index: usize,
        summary: &str,
        failed: bool,
        limited: bool,
        output: &str,
        now: Instant,
    ) {
        if let Some(block) = self.blocks.get_mut(index) {
            block.text.push_str(&format!(
                "\n  {}{summary}",
                if failed { "failed / " } else { "" }
            ));
            self.tools.finish(failed, limited, now);
            if let Some(tool) = self.tool_details.get_mut(&index) {
                tool.summary = summary.into();
                tool.status = if failed {
                    super::lab::model::Status::Failed
                } else if limited {
                    super::lab::model::Status::Warned
                } else {
                    super::lab::model::Status::Done
                };
                tool.detail = super::lab::model::Detail::Output(Self::tool_output(output));
                tool.elapsed_ms = self.tool_started.remove(&index).map_or(0, |started| {
                    now.saturating_duration_since(started).as_millis() as u64
                });
            }
        }
    }
}
