use super::{App, Key};
use crate::tui::{Feedback, state::Kind};

impl App {
    /// Handles a stop confirmation before normal panel navigation.
    pub(super) fn process_control(&mut self, key: Key) -> bool {
        let Some(panel) = &mut self.state.processes else {
            return false;
        };
        if panel.confirm_stop.is_some() {
            if key.code == 27 || key.ctrl() && key.code == 67 {
                panel.confirm_stop = None;
            } else if key.code == 13 && !key.ctrl() && !key.alt() && !key.shift() {
                let process = panel.confirm_stop.take().unwrap();
                if process.active() {
                    self.stop_process(&process.id);
                }
            }
            return true;
        }
        if key.ctrl() && !key.alt() && key.code == 68 {
            panel.confirm_stop = panel.selected_process().filter(|p| p.active()).cloned();
            return true;
        }
        false
    }

    /// Requests the selected runtime's stop without waiting or consuming output.
    pub(super) fn stop_process(&mut self, id: &str) {
        if id.is_empty() {
            self.warn("Usage: /terminal stop PROCESS_ID");
            return;
        }
        match self.process_observer.request_stop(id) {
            Ok(process) => {
                let status = if process.active() {
                    "Stop requested".into()
                } else {
                    process.label()
                };
                self.state.notify(Feedback::result(
                    Kind::Notice,
                    format!("{status} · {}", process.id),
                ));
            }
            Err(error) => self.warn(&error),
        }
        self.refresh_processes(true);
    }
}
