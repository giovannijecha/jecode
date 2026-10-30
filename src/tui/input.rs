use super::{App, Decoded, Feedback, Kind, selector::Purpose, terminal::Key};

impl App {
    pub(super) fn input(&mut self, decoded: Decoded) -> Result<bool, String> {
        // The quit question holds every key but reading the conversation.
        if self.state.quitting.is_some() {
            match &decoded {
                Decoded::Key(key) if key.alt() && !key.ctrl() && matches!(key.code, 35 | 36) => {}
                Decoded::Scroll(_) => {}
                Decoded::Key(key) => return Ok(self.quit_key(*key)),
                _ => return Ok(false),
            }
        }
        let decoded = match decoded {
            // Only an explicitly typed shortcut can decide a request.
            Decoded::Text(text) if self.state.permission.is_some() => {
                self.permission_text(&text);
                return Ok(false);
            }
            Decoded::Paste(text) if self.permission_guarded() => {
                self.continue_draft(&text);
                return Ok(false);
            }
            Decoded::Paste(_) | Decoded::Error if self.state.permission.is_some() => {
                return Ok(false);
            }
            // Terminals deliver a file drop as a paste of its paths.
            // The process panel keeps typed and pasted text out of the hidden
            // draft.
            Decoded::Text(text) if self.state.processes.is_some() => {
                self.process_text(&text, false);
                return Ok(false);
            }
            Decoded::Paste(text) if self.state.processes.is_some() => {
                self.process_text(&text, true);
                return Ok(false);
            }
            Decoded::Error if self.state.processes.is_some() => {
                self.warn("Input exceeds 1 MiB · terminal input rejected");
                return Ok(false);
            }
            Decoded::Text(text) | Decoded::Paste(text) if self.state.agents_panel.is_some() => {
                self.agents_text(&text);
                return Ok(false);
            }
            Decoded::Text(text) if self.state.ask_panel.is_some() => {
                self.ask_text(&text, false);
                return Ok(false);
            }
            Decoded::Paste(text) if self.state.ask_panel.is_some() => {
                self.ask_text(&text, true);
                return Ok(false);
            }
            // The Todo menu has nothing to type into.
            Decoded::Text(_) | Decoded::Paste(_) if self.state.todo_panel.is_some() => {
                return Ok(false);
            }
            Decoded::Error if self.state.agents_panel.is_some() => {
                self.warn("Input exceeds 1 MiB · agent filter unchanged");
                return Ok(false);
            }
            Decoded::Error if self.state.ask_panel.is_some() || self.state.todo_panel.is_some() => {
                self.warn("Input exceeds 1 MiB · panel input unchanged");
                return Ok(false);
            }
            Decoded::Paste(text) => {
                if self.state.selector.is_none()
                    && self.deletion_job.is_none()
                    && let Some(paths) = crate::attachments::paths::dropped(&text)
                {
                    self.state.feedback_input(true);
                    self.state.information = None;
                    self.attach(super::attach::Source::Paths(paths));
                    return Ok(false);
                }
                Decoded::Text(text)
            }
            decoded => decoded,
        };
        let key = match decoded {
            Decoded::Scroll(amount) => {
                self.scroll(amount);
                return Ok(false);
            }
            Decoded::Error => {
                self.state.feedback_input(true);
                self.warn("Input exceeds 1 MiB · insertion rejected, your draft was kept");
                return Ok(false);
            }
            Decoded::Text(text) | Decoded::Paste(text) => {
                if self.deletion_job.is_some() && self.state.selector.is_some() {
                    return Ok(false);
                }
                self.state.feedback_input(true);
                self.state.information = None;
                if let Some(selector) = &mut self.state.selector {
                    if selector.cancel_delete() && !selector.searchable {
                        return Ok(false);
                    }
                    if selector.accepts_text() {
                        if !selector.editor.insert(&text) {
                            self.warn("Input exceeds 1 MiB · insertion rejected");
                        } else {
                            self.state.selector.as_mut().unwrap().refresh();
                        }
                    } else if text.len() == 1
                        && let Some(number) = text.chars().next().and_then(|ch| ch.to_digit(10))
                    {
                        let choice = selector
                            .filtered
                            .get(number.saturating_sub(1) as usize)
                            .filter(|_| number > 0)
                            .map(|hit| selector.options[hit.index].choice);
                        if let Some(choice) = choice {
                            self.choose(choice)?;
                        }
                    }
                } else {
                    self.continue_draft(&text);
                }
                return Ok(false);
            }
            Decoded::Key(key) => key,
        };
        if self.direct_input_active() {
            return self.process_key(key);
        }
        self.state.feedback_input(false);
        if key.ctrl() && key.code == 81 {
            return Ok(self.quit());
        }
        if self.deletion_job.is_some() && (key.code == 27 || key.ctrl() && key.code == 67) {
            return Ok(false);
        }
        // Never reinterpret host scrolling shortcuts as editing or prompt history.
        if key.ctrl() && key.shift() && matches!(key.code, 33..=40) {
            return Ok(false);
        }
        if key.alt() && !key.ctrl() && matches!(key.code, 35 | 36) {
            self.renderer.scroll(if key.code == 36 {
                super::viewport::Scroll::Start
            } else {
                super::viewport::Scroll::End
            });
            return Ok(false);
        }
        if self.inspect_key(key) {
            return Ok(false);
        }
        if self.state.permission.is_some() {
            self.permission_key(key);
            return Ok(false);
        }
        if self.state.processes.is_some() {
            return self.process_key(key);
        }
        if self.state.agents_panel.is_some() {
            self.agents_key(key);
            return Ok(false);
        }
        if self.state.ask_panel.is_some() {
            self.ask_key(key);
            return Ok(false);
        }
        if self.state.todo_panel.is_some() {
            self.todo_key(key);
            return Ok(false);
        }
        if self.state.information.is_some() {
            if key.code == 27
                || key.ctrl() && key.code == 67
                || key.code == 13 && !key.ctrl() && !key.alt() && !key.shift()
            {
                self.state.information = None;
                return Ok(false);
            }
            if !key.ctrl() && !key.alt() && matches!(key.code, 33..=40) {
                super::view::scroll_information(&mut self.state, key.code);
                return Ok(false);
            }
            self.state.information = None;
        }
        if self.state.queue.is_editing() && (key.code == 27 || key.ctrl() && key.code == 67) {
            self.finish_draft_edit(false);
            return Ok(false);
        }
        if key.ctrl()
            && !key.alt()
            && !key.shift()
            && key.code == 83
            && (self.draft_menu_open() || self.state.queue.is_editing())
        {
            return self.send_paused_draft();
        }
        if key.ctrl() && key.code == 67 {
            if self.state.selector.is_some() {
                self.close_selector();
            } else if self.state.history.cancel(&mut self.state.editor) {
                self.edited();
            } else if self.worker.is_some() {
                self.stop();
            } else if !self.state.editor.text.is_empty() {
                self.state.editor.take();
                self.state.feedback_input(true);
                self.edited();
            }
            return Ok(false);
        }
        if key.code == 27 {
            if self.state.selector.is_some() {
                self.close_selector();
            } else if self.state.suggestions.panel {
                self.state.suggestions.dismiss();
            } else if self.state.history.cancel(&mut self.state.editor) {
                self.edited();
            } else if self.worker.is_some() {
                self.stop();
            }
            return Ok(false);
        }
        if self.state.selector.is_some() {
            return self.selector_key(key);
        }
        if !key.ctrl() && !key.alt() && matches!(key.code, 33 | 34) {
            self.renderer
                .scroll(super::viewport::Scroll::Page(key.code == 33));
            return Ok(false);
        }
        if key.alt() && key.code == 38 {
            self.state.feedback_input(true);
            self.open_drafts();
            return Ok(false);
        }
        if key.alt() && key.code == 40 {
            // Draft management has one panel; Alt+Down no longer discards text.
            return Ok(false);
        }
        if key.alt() && !key.ctrl() && key.code == 86 {
            // The clipboard is read only for this explicit shortcut.
            self.state.feedback_input(true);
            #[cfg(not(test))]
            let source = super::attach::Source::Clipboard;
            #[cfg(test)]
            let source = super::attach::Source::Image(
                self.clipboard_image
                    .take()
                    .unwrap_or_else(|| Err("The clipboard has no image".into())),
            );
            self.attach(source);
            return Ok(false);
        }
        if self.state.suggestions.visible && !key.ctrl() && !key.alt() {
            match key.code {
                38 | 40 => {
                    self.state.suggestions.move_selection(key.code == 38);
                    return Ok(false);
                }
                9 => {
                    if let Some(chosen) = self.state.suggestions.chosen() {
                        self.state.editor.replace(chosen.into());
                        self.state.feedback_input(true);
                        self.edited();
                    }
                    return Ok(false);
                }
                _ => {}
            }
        }
        match key.code {
            13 if !key.shift() && !key.alt() && !key.ctrl() => {
                self.typed = None;
                return self.submit();
            }
            112 if self.checkpoint_job.is_none() && self.deletion_job.is_none() => self.help(),
            80 | 78 if key.ctrl() => {
                if self.state.queue.is_editing() {
                    self.warn(
                        "Save this draft with Enter or cancel with Esc before browsing history",
                    );
                    return Ok(false);
                }
                self.state
                    .history
                    .navigate(&mut self.state.editor, key.code == 80);
                self.state.feedback_input(true);
                self.edited();
            }
            38 | 40 if !key.ctrl() => {
                self.state
                    .editor
                    .vertical(key.code == 38, super::view::input_width(self.state.width));
                self.edited();
            }
            _ => self.edit_draft(key),
        }
        Ok(false)
    }

    /// Text typed into the chat draft, also while a request that arrived
    /// during the typing guards its answer.
    pub(super) fn continue_draft(&mut self, text: &str) {
        if !self.state.editor.insert(text) {
            self.warn("Input exceeds 1 MiB · insertion rejected, your draft was kept");
        } else {
            self.typed = Some(std::time::Instant::now());
            self.edited();
            self.state.history.edited();
        }
    }

    /// An editing key on the chat draft.
    pub(super) fn edit_draft(&mut self, key: Key) {
        let before = self.state.editor.text.clone();
        if !self.state.editor.key(key) {
            self.warn("Input exceeds 1 MiB · insertion rejected, your draft was kept");
        }
        if self.state.editor.text != before {
            self.typed = Some(std::time::Instant::now());
            self.state.feedback_input(true);
            self.edited();
            self.state.history.edited();
        }
    }

    fn scroll(&mut self, amount: i16) {
        if self.deletion_job.is_some() {
            return;
        }
        if self.state.processes.is_some() {
            self.process_scroll(amount);
        } else if self.state.agents_panel.is_some() {
            self.agents_scroll(amount);
        } else if self.state.ask_panel.is_some() {
            self.ask_scroll(amount);
        } else if self.state.todo_panel.is_some() {
            self.todo_scroll(amount);
        } else if self.state.information.is_some() {
            for _ in 0..amount.unsigned_abs().min(30) {
                super::view::scroll_information(&mut self.state, if amount < 0 { 38 } else { 40 });
            }
        } else if let Some(menu) = &mut self.state.selector {
            for _ in 0..amount.unsigned_abs().min(30) {
                menu.move_selection(amount < 0);
            }
        } else {
            self.renderer
                .scroll(super::viewport::Scroll::Rows(amount.into()));
        }
    }

    fn selector_key(&mut self, key: Key) -> Result<bool, String> {
        if self.deletion_job.is_some() {
            return Ok(false);
        }
        if key.code == 68 && key.ctrl() && !key.alt() && !key.shift() {
            if !self.revoke_grant() {
                self.state.selector.as_mut().unwrap().toggle_delete();
            }
            return Ok(false);
        }
        let menu = self.state.selector.as_mut().unwrap();
        match key.code {
            38 | 40 => menu.move_selection(key.code == 38),
            13 if !key.shift() && !key.alt() && !key.ctrl() => {
                if matches!(menu.purpose, Purpose::Key) {
                    self.save_key();
                } else if matches!(menu.purpose, Purpose::AutoCompact) {
                    self.save_auto_compact();
                } else if let Some(choice) = menu.chosen() {
                    self.choose(choice)?;
                }
            }
            _ if menu.accepts_text() => {
                let before = menu.editor.text.clone();
                menu.editor.key(key);
                if before != menu.editor.text {
                    menu.refresh();
                    self.state.feedback_input(true);
                }
            }
            _ => {}
        }
        Ok(false)
    }

    pub(super) fn edited(&mut self) {
        if self.state.queue.is_editing() {
            self.state.suggestions.dismiss();
        } else {
            self.state.suggestions.refresh(&self.state.editor.text);
        }
    }
    pub(super) fn warn(&mut self, text: &str) {
        self.state.notify(Feedback::result(
            Kind::Warning,
            self.archive.redactor.text(text),
        ));
    }
    pub(super) fn stop(&mut self) {
        if let Some(worker) = &self.worker {
            self.collaboration_auto_resume = false;
            self.collaboration_return_pending = false;
            worker.cancel();
            if let Some(activity) = &mut self.state.activity {
                activity.stopping = true;
            }
            self.state.clear_notice();
            self.state.status = "Stopping...".into();
        }
    }
    pub(super) fn close_selector(&mut self) {
        if self
            .state
            .selector
            .as_mut()
            .is_some_and(|menu| menu.cancel_delete())
        {
            return;
        }
        if self.copy_selector_open() {
            self.state.selector = None;
            self.copy_targets.clear();
            return;
        }
        if self.draft_menu_open() {
            self.state.selector = None;
            return;
        }
        if self.close_provider_panel() || self.close_permissions_menu() {
            return;
        }
        drop(self.job.take());
        if self.settings_parent.is_some() {
            self.cancel_setting();
            return;
        }
        if self
            .state
            .selector
            .as_ref()
            .is_some_and(|menu| matches!(menu.purpose, Purpose::Sessions { .. }))
        {
            self.state.selector = None;
            self.state.status = "Ready".into();
            return;
        }
        self.state.selector = None;
        self.local_finish_quiet(
            Kind::Warning,
            "Selection cancelled · previous settings kept",
            vec![],
        );
    }
}
