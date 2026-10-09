use crate::session::commands::{self, COMMANDS, Match};

#[derive(Default)]
pub struct Suggestions {
    pub matches: Vec<Match>,
    pub selected: usize,
    pub visible: bool,
    pub panel: bool,
}

impl Suggestions {
    pub fn refresh(&mut self, draft: &str) {
        self.panel = draft.starts_with('/');
        self.visible = self.panel && !draft.chars().any(char::is_whitespace);
        self.matches = if self.visible {
            commands::suggestions(draft)
        } else {
            vec![]
        };
        self.selected = 0;
    }
    pub fn dismiss(&mut self) {
        self.visible = false;
        self.panel = false;
    }
    pub fn move_selection(&mut self, up: bool) {
        if self.matches.is_empty() {
            return;
        }
        self.selected = if up {
            self.selected.saturating_sub(1)
        } else {
            (self.selected + 1).min(self.matches.len() - 1)
        };
    }
    pub fn chosen(&self) -> Option<&'static str> {
        self.matches
            .get(self.selected)
            .map(|matched| COMMANDS[matched.index].name)
    }
}
