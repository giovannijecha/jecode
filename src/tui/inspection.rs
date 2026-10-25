use super::{App, terminal::Key};

impl App {
    pub(super) fn inspect_key(&mut self, key: Key) -> bool {
        if !key.ctrl() || key.alt() || key.shift() || key.code != 79 {
            return false;
        }
        if self.idle() && !super::view::panel_open(&self.state) && !self.state.queue.is_editing() {
            self.state.toggle_tools();
        }
        true
    }
}
