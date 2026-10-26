use super::{Item, State};
use std::{collections::VecDeque, ops::Range};

const RECENT_TOOL_CHANGES: usize = 64;

#[derive(Default)]
pub struct ToolDetails {
    pub turn: Option<Range<usize>>,
    pub active: bool,
    pub expanded: bool,
    pub(super) turn_id: u64,
    /// Equal consecutive starts share a record, even across a large read batch.
    changes: VecDeque<(usize, u64)>,
}

impl ToolDetails {
    /// Returns the earliest item affected by every unseen revision. When a
    /// layout is older than the bounded history, its caller rebuilds all rows.
    pub(in crate::tui) fn changed_since(&self, revisions: u64) -> Option<usize> {
        let mut remaining = revisions;
        let mut first = None;
        for &(index, count) in self.changes.iter().rev() {
            first = Some(first.map_or(index, |earlier: usize| earlier.min(index)));
            remaining = remaining.saturating_sub(count);
            if remaining == 0 {
                return first;
            }
        }
        None
    }
}

impl State {
    pub(super) fn cache_chapter(&mut self, index: usize) {
        if self.items[index].groupable() {
            let start = super::rows::chapter_start(&self.items, index);
            if let Item::Tool { presentation, .. } = &mut self.items[index] {
                presentation.chapter_start = Some(start);
            }
        }
    }

    /// Item removal changes indices; reconstruct derived chapter starts in a
    /// forward pass, retaining the original execution-turn identities.
    pub fn reindex_chapters(&mut self) {
        for index in 0..self.items.len() {
            if let Item::Tool { presentation, .. } = &mut self.items[index] {
                presentation.chapter_start = None;
            }
            self.cache_chapter(index);
        }
    }

    fn tool_changed_from(&mut self, first: usize) {
        self.tool_revision = self.tool_revision.wrapping_add(1);
        if let Some((index, count)) = self.tool_details.changes.back_mut()
            && *index == first
        {
            *count = count.saturating_add(1);
            return;
        }
        if self.tool_details.changes.len() == RECENT_TOOL_CHANGES {
            self.tool_details.changes.pop_front();
        }
        self.tool_details.changes.push_back((first, 1));
    }

    pub fn begin_tool_turn(&mut self) {
        self.reset_tool_details();
        self.tool_details.turn_id = self.tool_details.turn_id.wrapping_add(1);
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
        let mut count = 0;
        for item in &mut self.items[turn] {
            if let Item::Tool {
                presentation,
                result: Some(_),
                ..
            } = item
            {
                presentation.expanded = expanded;
                count += 1;
            }
        }
        self.tool_details.expanded = expanded;
        self.tool_changed_from(first);
        self.changed();
        Some((expanded, count))
    }

    /// A changed call updates the chapter heading at its first member without
    /// invalidating earlier conversation blocks.
    pub(super) fn update_chapters(&mut self, changed: &[usize]) {
        let start = changed
            .iter()
            .filter(|&&index| self.items[index].groupable())
            .map(|&index| super::rows::chapter_start(&self.items, index))
            .min();
        if let Some(start) = start {
            self.tool_changed_from(start);
        }
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
            self.tool_changed_from(first);
            self.changed();
        }
    }
}
