use super::{App, Feedback, Kind, terminal::Key};

impl App {
    pub(super) fn inspect_key(&mut self, key: Key) -> bool {
        if !key.ctrl() || key.alt() || key.shift() || key.code != 79 {
            return false;
        }
        if self.idle()
            && !super::view::panel_open(&self.state)
            && !self.state.queue.is_editing()
            && let Some((expanded, count)) = self.state.toggle_tools()
        {
            self.state.notify(Feedback::result(
                Kind::Notice,
                format!(
                    "Tool details {} · {count} {}",
                    if expanded { "expanded" } else { "collapsed" },
                    if count == 1 { "tool" } else { "tools" }
                ),
            ));
            return true;
        }
        true
    }
}
