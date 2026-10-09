use super::{Item, State};

impl State {
    pub fn select_tool(&mut self, index: Option<usize>) {
        if let Some(Item::Tool { presentation, .. }) =
            self.tool_focus.and_then(|index| self.items.get_mut(index))
        {
            presentation.selected = false;
        }
        self.tool_focus =
            index.filter(|&index| matches!(self.items.get(index), Some(Item::Tool { .. })));
        if let Some(Item::Tool { presentation, .. }) =
            self.tool_focus.and_then(|index| self.items.get_mut(index))
        {
            presentation.selected = true;
        }
        self.tool_revision = self.tool_revision.wrapping_add(1);
        self.changed();
    }

    pub fn move_tool(&mut self, up: bool) {
        let Some(current) = self.tool_focus else {
            return;
        };
        let next = if up {
            self.items[..current]
                .iter()
                .rposition(|item| matches!(item, Item::Tool { .. }))
        } else {
            self.items
                .iter()
                .enumerate()
                .skip(current + 1)
                .find(|(_, item)| matches!(item, Item::Tool { .. }))
                .map(|(index, _)| index)
        };
        if let Some(next) = next {
            self.select_tool(Some(next));
        }
    }

    pub fn toggle_tool(&mut self) {
        if let Some(Item::Tool { presentation, .. }) =
            self.tool_focus.and_then(|index| self.items.get_mut(index))
        {
            presentation.expanded = Some(presentation.expanded != Some(true));
            self.tool_revision = self.tool_revision.wrapping_add(1);
            self.changed();
        }
    }
}
