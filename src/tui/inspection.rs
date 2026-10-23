use super::{App, terminal::Key, viewport::Scroll};

impl App {
    pub(super) fn inspect_key(&mut self, key: Key) -> bool {
        if !key.ctrl() || key.alt() || key.shift() || key.code != 79 {
            return false;
        }
        if self.idle()
            && !super::view::panel_open(&self.state)
            && !self.state.queue.is_editing()
            && let Some((expanded, first)) = self.state.toggle_tools()
        {
            self.renderer.scroll(if expanded {
                Scroll::Reveal(first + 1)
            } else {
                Scroll::End
            });
        }
        true
    }
}
