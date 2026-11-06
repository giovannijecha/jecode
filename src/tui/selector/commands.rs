//! The commands sheet's list: the catalogue `/` shows, in the same groups and order.

use super::{Choice, OptionRow, Purpose, Selector, row};
use crate::session::commands::{COMMANDS, GROUPS};

impl Selector {
    /// The rows carry no fact, so a filter matches names alone, as `/` does.
    pub fn commands(waits: impl Fn(&str) -> bool) -> Self {
        let options: Vec<OptionRow> = GROUPS
            .iter()
            .flat_map(|group| {
                COMMANDS
                    .iter()
                    .enumerate()
                    .filter(move |(_, command)| command.group() == *group)
            })
            .map(|(index, command)| row(command.name, "", Choice::Command(index)))
            .collect();
        let later = vec![false; options.len()];
        let mut menu = Self::new("Commands", Purpose::Commands { later }, options);
        menu.hold_commands(waits);
        menu
    }

    /// Marks what must wait for the running work, moving the focus off it. Returns
    /// whether anything changed.
    pub fn hold_commands(&mut self, waits: impl Fn(&str) -> bool) -> bool {
        let held: Vec<bool> = self.options.iter().map(|row| waits(&row.name)).collect();
        let Purpose::Commands { later } = &mut self.purpose else {
            return false;
        };
        if *later == held {
            return false;
        }
        *later = held;
        if !self.selectable(self.selected) {
            self.selected = self.first_selectable();
        }
        true
    }

    /// Whether the row at `position` of the filtered list can be chosen.
    pub fn selectable(&self, position: usize) -> bool {
        let Purpose::Commands { later } = &self.purpose else {
            return true;
        };
        self.filtered
            .get(position)
            .is_some_and(|hit| !later[hit.index])
    }

    pub(super) fn first_selectable(&self) -> usize {
        (0..self.filtered.len())
            .find(|position| self.selectable(*position))
            .unwrap_or(0)
    }
}
