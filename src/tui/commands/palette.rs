//! `Ctrl+G`: the commands as a sheet. Choosing one runs it without touching the draft,
//! whichever conversation that draft ends up in.

use super::super::{App, selector::Selector};
use super::{immediate, menu};
use crate::{attachments::Prompt, session::commands::COMMANDS};

/// Whether a command has to wait: what `submit` would queue, here never queued.
fn waits(name: &str, busy: bool, turn: bool) -> bool {
    busy && !immediate(name) && !(turn && menu(name))
}

impl App {
    pub(in crate::tui) fn open_commands(&mut self) {
        self.state.clear_notice();
        let (busy, turn) = (self.busy(), self.held.is_some());
        self.state.selector = Some(Selector::commands(|name| waits(name, busy, turn)));
    }

    pub(in crate::tui) fn run_command(&mut self, index: usize) -> Result<bool, String> {
        self.state.selector = None;
        self.from_commands = true;
        self.state.sent();
        self.dispatch(Prompt::plain(COMMANDS[index].name))
    }

    /// Keeps the rows honest as work starts and ends under an open sheet.
    pub(in crate::tui) fn sync_commands(&mut self) -> bool {
        let (busy, turn) = (self.busy(), self.held.is_some());
        self.state
            .selector
            .as_mut()
            .is_some_and(|menu| menu.hold_commands(|name| waits(name, busy, turn)))
    }

    /// Esc out of what the sheet opened goes back to it, one level at a time. Any other
    /// way out ends in the composer.
    pub(in crate::tui) fn follow_commands(&mut self, escaped: bool) {
        let state = &self.state;
        let open = state.selector.is_some()
            || state.information.is_some()
            || self.checkpoint_job.is_some()
            || state.processes.is_some()
            || state.agents_panel.is_some()
            || state.ask_panel.is_some()
            || state.todo_panel.is_some();
        if !self.from_commands || open {
            return;
        }
        self.from_commands = false;
        if escaped {
            self.open_commands();
        }
    }
}

/// What waits while a turn runs, for the specimen.
#[cfg(test)]
pub(in crate::tui) fn busy_waits(name: &str) -> bool {
    waits(name, true, true)
}

#[cfg(test)]
#[path = "palette_tests.rs"]
mod tests;
