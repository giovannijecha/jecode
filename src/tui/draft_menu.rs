use super::{
    App,
    drafts::Slot,
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

    /// Rebuilds the menu, keeping the selected draft when it is still pending
    /// and its row otherwise.
    fn refresh_drafts(&mut self, selected: usize, slot: Option<Slot>) {
        let mut menu = Selector::drafts(&self.state.queue);
        if let Some(previous) = self.state.selector.take() {
            menu.searchable |= previous.searchable;
            menu.editor = previous.editor;
            menu.refresh();
        }
        let kept = menu.filtered.iter().position(|hit| {
            matches!(menu.options[hit.index].choice, Choice::Draft(draft) if Some(draft) == slot)
        });
        menu.selected = kept.unwrap_or(selected.min(menu.filtered.len().saturating_sub(1)));
        self.state.selector = Some(menu);
    }

    pub(super) fn refresh_draft_menu(&mut self) {
        if self.draft_menu_open() {
            let menu = self.state.selector.as_ref().unwrap();
            let slot = match menu.chosen() {
                Some(Choice::Draft(slot)) => Some(slot),
                _ => None,
            };
            self.refresh_drafts(menu.selected, slot);
        }
    }

    pub(super) fn choose_draft(&mut self, slot: Slot, discard: bool) {
        if discard {
            let selected = self.state.selector.as_ref().map_or(0, |menu| menu.selected);
            if !self.state.queue.discard(slot) {
                self.warn("This message was already sent.");
            }
            self.refresh_drafts(selected, None);
            self.capture_input();
            return;
        }
        self.state.history.cancel(&mut self.state.editor);
        match self.state.queue.begin_edit(slot, &mut self.state.editor) {
            Ok(()) => {
                self.state.selector = None;
                self.edited();
                self.capture_input();
            }
            Err(error) => {
                self.warn(&error);
                self.refresh_draft_menu();
            }
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
        let slot = self.state.queue.editing().or_else(|| {
            self.state
                .selector
                .as_ref()
                .and_then(|menu| match menu.chosen() {
                    Some(Choice::Draft(slot)) => Some(slot),
                    _ => None,
                })
        });
        let index = match slot {
            None => return Ok(false),
            Some(Slot::Queued(_)) => {
                self.warn("This draft is already queued for automatic sending.");
                return Ok(false);
            }
            Some(Slot::Paused(index)) => index,
        };
        let text = if self.state.queue.is_editing() {
            self.state.editor.prompt()
        } else {
            self.state.queue.paused[index].prompt()
        };
        if text.is_empty() {
            self.warn("The draft is empty. Edit it or discard it in /drafts.");
            return Ok(false);
        }
        let busy = self.worker.is_some() || self.deletion_job.is_some();
        if busy && let Err(error) = self.state.queue.push(text.clone()) {
            self.warn(&error);
            return Ok(false);
        }
        if self.state.queue.is_editing() {
            self.state.queue.cancel_edit(&mut self.state.editor);
        }
        self.state.queue.take(Slot::Paused(index));
        self.state.selector = None;
        self.edited();
        // While busy, saving the input lets the active turn accept the draft.
        self.capture_input();
        if busy {
            return Ok(false);
        }
        self.dispatch(text)
    }

    pub(super) fn keep_failed_prompt(&mut self, text: crate::attachments::Prompt) {
        self.state.queue.keep_failed(text);
        self.edited();
        self.capture_input();
    }
}

#[cfg(test)]
mod tests;
