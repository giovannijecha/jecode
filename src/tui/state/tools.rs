use super::{Item, State};
use std::ops::Range;

#[derive(Default)]
pub struct ToolDetails {
    pub turn: Option<Range<usize>>,
    pub active: bool,
    pub expanded: bool,
    pub changed_from: usize,
}

impl State {
    pub fn begin_tool_turn(&mut self) {
        self.reset_tool_details();
        self.tool_details.turn = Some(self.items.len()..self.items.len());
        self.tool_details.active = true;
    }

    pub fn finish_tool_turn(&mut self) {
        if self.tool_details.active {
            if let Some(turn) = &mut self.tool_details.turn {
                turn.end = self.items.len();
            }
            self.tool_details.active = false;
            self.changed();
        }
    }

    pub fn tool_details_available(&self) -> bool {
        !self.tool_details.active
            && self.tool_details.turn.as_ref().is_some_and(|turn| {
                self.items[turn.clone()].iter().any(|item| {
                    matches!(
                        item,
                        Item::Tool {
                            result: Some(_),
                            ..
                        }
                    )
                })
            })
    }

    pub fn toggle_tools(&mut self) -> Option<(bool, usize)> {
        if !self.tool_details_available() {
            return None;
        }
        let turn = self.tool_details.turn.clone()?;
        let first = turn.clone().find(|&index| {
            matches!(
                self.items[index],
                Item::Tool {
                    result: Some(_),
                    ..
                }
            )
        })?;
        let expanded = !self.tool_details.expanded;
        for item in &mut self.items[turn] {
            if let Item::Tool {
                presentation,
                result: Some(_),
                ..
            } = item
            {
                presentation.expanded = expanded;
            }
        }
        self.tool_details.expanded = expanded;
        self.tool_details.changed_from = first;
        self.tool_revision = self.tool_revision.wrapping_add(1);
        self.changed();
        Some((expanded, first))
    }

    pub fn reset_tool_details(&mut self) {
        let mut first = None;
        for (index, item) in self.items.iter_mut().enumerate() {
            if let Item::Tool { presentation, .. } = item
                && presentation.expanded
            {
                presentation.expanded = false;
                first.get_or_insert(index);
            }
        }
        self.tool_details.turn = None;
        self.tool_details.active = false;
        self.tool_details.expanded = false;
        if let Some(first) = first {
            self.tool_details.changed_from = first;
            self.tool_revision = self.tool_revision.wrapping_add(1);
            self.changed();
        }
    }
}
