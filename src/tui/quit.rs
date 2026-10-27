use super::{App, terminal::Key};

impl App {
    /// Ctrl+Q quits at once when idle; a running turn or background process
    /// asks first. Returns whether to quit.
    pub(super) fn quit(&mut self) -> bool {
        let processes: Vec<_> = self
            .process_observer
            .processes()
            .unwrap_or_default()
            .into_iter()
            .filter(|process| process.active())
            .map(|process| {
                let command = process.command.lines().next().unwrap_or_default();
                (
                    self.archive.redactor.text(command),
                    format!("background process {}", process.id),
                )
            })
            .collect();
        if self.worker.is_none() && processes.is_empty() {
            self.stop();
            return true;
        }
        self.state.quitting = Some(processes);
        false
    }

    /// Enter or a second Ctrl+Q stops the work and quits; Esc or Ctrl+C
    /// cancels. Other keys wait for the answer.
    pub(super) fn quit_key(&mut self, key: Key) -> bool {
        let plain = !key.ctrl() && !key.alt() && !key.shift();
        if plain && key.code == 13 || key.ctrl() && key.code == 81 {
            self.state.quitting = None;
            self.stop();
            return true;
        }
        if key.code == 27 || key.ctrl() && key.code == 67 {
            self.state.quitting = None;
        }
        false
    }
}
