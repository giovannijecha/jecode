use super::{
    App,
    selector::{Choice, Purpose, Selector},
};

impl App {
    pub(super) fn draft_menu_open(&self) -> bool {
        self.state
            .selector
            .as_ref()
            .is_some_and(|menu| matches!(menu.purpose, Purpose::Drafts { .. }))
    }

    pub(super) fn open_drafts(&mut self) {
        if self.state.queue.is_editing() {
            self.warn("Save this draft with Enter or cancel with Esc first.");
            return;
        }
        self.state.history.cancel(&mut self.state.editor);
        self.state.information = None;
        self.state.suggestions.dismiss();
        self.state.selector = Some(Selector::drafts(&self.state.queue));
    }

    fn refresh_drafts(&mut self, selected: usize) {
        let mut menu = Selector::drafts(&self.state.queue);
        if let Some(previous) = self.state.selector.take() {
            menu.searchable |= previous.searchable;
            menu.editor = previous.editor;
            menu.refresh();
        }
        menu.selected = selected.min(menu.filtered.len().saturating_sub(1));
        self.state.selector = Some(menu);
    }

    pub(super) fn refresh_draft_menu(&mut self) {
        if self.draft_menu_open() {
            let selected = self.state.selector.as_ref().unwrap().selected;
            self.refresh_drafts(selected);
        }
    }

    pub(super) fn choose_draft(&mut self, index: usize, discard: bool) {
        if discard {
            let selected = self.state.selector.as_ref().map_or(0, |menu| menu.selected);
            self.state.queue.discard(index);
            self.refresh_drafts(selected);
            self.capture_input();
            return;
        }
        self.state.history.cancel(&mut self.state.editor);
        match self.state.queue.begin_edit(index, &mut self.state.editor) {
            Ok(()) => {
                self.state.selector = None;
                self.edited();
            }
            Err(error) => self.warn(&error),
        }
    }

    pub(super) fn finish_draft_edit(&mut self, save: bool) {
        let index = self.state.queue.edit_index().unwrap_or(0);
        if save {
            self.state.queue.save_edit(&mut self.state.editor);
        } else {
            self.state.queue.cancel_edit(&mut self.state.editor);
        }
        self.edited();
        self.open_drafts();
        if let Some(menu) = &mut self.state.selector {
            menu.selected = index.min(menu.filtered.len().saturating_sub(1));
        }
        self.capture_input();
    }

    pub(super) fn send_paused_draft(&mut self) -> Result<bool, String> {
        let index = self.state.queue.edit_index().or_else(|| {
            self.state
                .selector
                .as_ref()
                .and_then(|menu| match menu.chosen() {
                    Some(Choice::Draft(index)) => Some(index),
                    _ => None,
                })
        });
        let Some(index) = index else {
            return Ok(false);
        };
        let queued = self.state.queue.messages.len();
        if index < queued {
            self.warn("This draft is already queued for automatic sending.");
            return Ok(false);
        }
        let text = if self.state.queue.is_editing() {
            self.state.editor.text.clone()
        } else {
            self.state.queue.paused[index - queued].text.clone()
        };
        if text.trim().is_empty() {
            self.warn("The draft is empty. Edit it or discard it in /drafts.");
            return Ok(false);
        }
        if self.worker.is_some() || self.deletion_job.is_some() {
            if let Err(error) = self.state.queue.push(text) {
                self.warn(&error);
                return Ok(false);
            }
            // Adding an automatic message shifts the paused portion by one slot.
            if self.state.queue.is_editing() {
                self.state.queue.cancel_edit(&mut self.state.editor);
            }
            self.state.queue.take(index + 1);
            self.state.selector = None;
            self.edited();
            self.capture_input();
            return Ok(false);
        }
        if self.state.queue.is_editing() {
            self.state.queue.cancel_edit(&mut self.state.editor);
        }
        self.state.queue.take(index);
        self.state.selector = None;
        self.edited();
        self.capture_input();
        self.dispatch(text)
    }

    pub(super) fn keep_failed_prompt(&mut self, text: String) {
        self.state.queue.messages.push_front(text);
        self.state.queue.pause();
        self.edited();
        self.capture_input();
    }
}

#[cfg(test)]
mod tests;
