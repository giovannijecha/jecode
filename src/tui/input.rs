use super::{App, Decoded, Feedback, Kind, selector::Purpose, terminal::Key};

impl App {
    pub(super) fn input(&mut self, decoded: Decoded) -> Result<bool, String> {
        let key = match decoded {
            Decoded::Scroll(amount) => {
                self.scroll(amount);
                return Ok(false);
            }
            Decoded::Error => {
                self.state.feedback_input(true);
                self.warn("Input exceeds 1 MiB. Insertion rejected; your draft was kept.");
                return Ok(false);
            }
            Decoded::Text(text) => {
                if self.state.tool_focus.is_some() {
                    self.state.select_tool(None);
                }
                if self.deletion_job.is_some() && self.state.selector.is_some() {
                    return Ok(false);
                }
                self.state.feedback_input(true);
                self.state.information = None;
                if let Some(selector) = &mut self.state.selector {
                    if selector.cancel_delete() && !selector.searchable {
                        return Ok(false);
                    }
                    if selector.searchable || matches!(selector.purpose, Purpose::Key) {
                        if !selector.editor.insert(&text) {
                            self.warn("Input exceeds 1 MiB. Insertion rejected.");
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
                    if !self.state.editor.insert(&text) {
                        self.warn("Input exceeds 1 MiB. Insertion rejected; your draft was kept.");
                    } else {
                        self.edited();
                        self.state.history.edited();
                    }
                }
                return Ok(false);
            }
            Decoded::Key(key) => key,
        };
        self.state.feedback_input(false);
        if key.ctrl() && key.code == 81 {
            self.stop();
            return Ok(true);
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
        if self.state.information.is_some() {
            if key.code == 27
                || key.ctrl() && key.code == 67
                || key.code == 13 && !key.ctrl() && !key.alt() && !key.shift()
            {
                self.state.information = None;
                return Ok(false);
            }
            if !key.ctrl() && !key.alt() && matches!(key.code, 33..=36 | 38 | 40) {
                super::view::scroll_information(&mut self.state, key.code);
                return Ok(false);
            }
            self.state.information = None;
        }
        if self.state.selector.is_none() && self.inspect_key(key) {
            return Ok(false);
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
            } else if self.renderer.back_to_bottom() {
                // Return to the latest output before interpreting Esc as a stop.
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
            13 if !key.shift() && !key.alt() && !key.ctrl() => return self.submit(),
            112 if self.worker.is_none() => self.help(),
            80 | 78 if key.ctrl() => {
                if self.state.queue.is_editing() {
                    self.warn(
                        "Save this draft with Enter or cancel with Esc before browsing history.",
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
                self.state.editor.vertical(
                    key.code == 38,
                    super::view::input_width(self.state.width, self.state.suggestions.panel),
                );
                self.edited();
            }
            _ => {
                let before = self.state.editor.text.clone();
                if !self.state.editor.key(key) {
                    self.warn("Input exceeds 1 MiB. Insertion rejected; your draft was kept.");
                }
                if self.state.editor.text != before {
                    self.state.feedback_input(true);
                    self.edited();
                    self.state.history.edited();
                }
            }
        }
        Ok(false)
    }

    fn scroll(&mut self, amount: i16) {
        if self.deletion_job.is_some() {
            return;
        }
        if self.state.information.is_some() {
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
        let menu = self.state.selector.as_mut().unwrap();
        if key.code == 68 && key.ctrl() && !key.alt() && !key.shift() {
            menu.toggle_delete();
            return Ok(false);
        }
        match key.code {
            38 | 40 => menu.move_selection(key.code == 38),
            13 if !key.shift() && !key.alt() && !key.ctrl() => {
                if matches!(menu.purpose, Purpose::Key) {
                    self.save_key();
                } else if let Some(choice) = menu.chosen() {
                    self.choose(choice)?;
                }
            }
            _ if menu.searchable || matches!(menu.purpose, Purpose::Key) => {
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
    fn stop(&mut self) {
        if let Some(worker) = &self.worker {
            worker.cancel();
            if let Some(activity) = &mut self.state.activity {
                activity.stopping = true;
                activity.label = "Stopping";
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
