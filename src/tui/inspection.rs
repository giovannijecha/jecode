use super::{App, state::Item, terminal::Key, viewport::Scroll};

impl App {
    pub(super) fn inspect_key(&mut self, key: Key) -> bool {
        if key.alt() && !key.ctrl() && key.code == 84 {
            let next = if self.state.tool_focus.is_some() {
                None
            } else {
                self.state
                    .items
                    .iter()
                    .rposition(|item| matches!(item, Item::Tool { .. }))
            };
            self.state.select_tool(next);
            if next.is_some() {
                self.state.suggestions.dismiss();
            } else {
                self.edited();
            }
            self.reveal_tool();
            return true;
        }
        if self.state.tool_focus.is_none() {
            return false;
        }
        if key.code == 27
            || key.ctrl() && key.code == 67
            || key.code == 9 && !key.alt() && !key.ctrl()
        {
            self.state.select_tool(None);
            self.edited();
            return true;
        }
        if !key.alt() && !key.ctrl() && !key.shift() {
            match key.code {
                38 | 40 => self.state.move_tool(key.code == 38),
                13 => self.state.toggle_tool(),
                _ => {
                    if !matches!(key.code, 33 | 34) {
                        self.state.select_tool(None);
                        self.edited();
                    }
                    return false;
                }
            }
            self.reveal_tool();
            return true;
        }
        if !(key.alt() && !key.ctrl() && matches!(key.code, 35 | 36)) {
            self.state.select_tool(None);
            self.edited();
        }
        false
    }

    fn reveal_tool(&mut self) {
        if let Some(index) = self.state.tool_focus {
            self.renderer.scroll(Scroll::Reveal(index + 1));
        }
    }
}
