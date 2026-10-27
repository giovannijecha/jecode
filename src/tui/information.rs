use crate::session::commands::COMMANDS;

/// A read-only panel of labeled rows, in one or more sections. A row with an
/// empty value starts a group.
pub(super) struct Information {
    pub title: String,
    pub context: String,
    pub sections: Vec<Section>,
    pub section: usize,
    pub offset: usize,
}

pub(super) struct Section {
    pub name: String,
    pub aside: String,
    pub details: Vec<(String, String)>,
    /// Labels are key names, drawn as keycaps.
    pub keys: bool,
}

/// Keyboard controls by group, for `/help`. Several keys join with ` · `.
const KEYS: &[(&str, &[(&str, &str)])] = &[
    (
        "Composer",
        &[
            (
                "Enter",
                "Send · while working, joins the turn before its next model request",
            ),
            ("Ctrl+J", "New line · Shift/Alt+Enter when supported"),
            ("Ctrl+P/N", "Sent prompt history · commands are excluded"),
            ("Alt+↑", "Open pending drafts · edit or discard one message"),
        ],
    ),
    (
        "Conversation",
        &[
            (
                "PgUp/PgDn",
                "Scroll, also with the mouse wheel · new output keeps your reading position",
            ),
            (
                "Alt+Home/End",
                "Conversation start / follow latest output · Esc returns to bottom",
            ),
            ("Ctrl+O", "Expand/collapse the last finished turn's tools"),
            ("Esc · Ctrl+C", "Stop work · close panel / clear draft"),
            ("Ctrl+Q", "Quit, asking first while work runs"),
        ],
    ),
    (
        "In panels",
        &[
            (
                "Ctrl+D",
                "/resume, /drafts: mark deletion · /terminal: mark a process to stop · Enter confirms, Esc cancels",
            ),
            (
                "Ctrl+S",
                "/drafts: send a paused draft · joins the active turn while working",
            ),
            (
                "Tab",
                "/terminal output: switch Output and Errors · End follows",
            ),
        ],
    ),
];

/// The `/help` rows as recorded: every command, then the keyboard controls.
pub(super) fn help_details() -> Vec<(String, String)> {
    let mut details: Vec<(String, String)> = COMMANDS
        .iter()
        .map(|command| (command.name.into(), command.description(true).into()))
        .collect();
    details.extend(
        KEYS.iter()
            .flat_map(|(_, rows)| rows.iter())
            .map(|(key, text)| ((*key).into(), (*text).into())),
    );
    details
}

impl Information {
    pub fn new(title: String, details: Vec<(String, String)>) -> Self {
        Self {
            title,
            context: String::new(),
            sections: vec![Section {
                name: String::new(),
                aside: String::new(),
                details,
                keys: false,
            }],
            section: 0,
            offset: 0,
        }
    }

    /// `/help`: commands and keys as sections.
    pub fn help() -> Self {
        let commands: Vec<_> = COMMANDS
            .iter()
            .map(|command| (command.tui_usage.into(), command.description(true).into()))
            .collect();
        let mut keys = Vec::new();
        for (group, rows) in KEYS {
            keys.push(((*group).into(), String::new()));
            keys.extend(
                rows.iter()
                    .map(|(key, text)| ((*key).into(), (*text).into())),
            );
        }
        let count = keys.iter().filter(|(_, text)| !text.is_empty()).count();
        Self {
            title: "Help".into(),
            context: String::new(),
            sections: vec![
                Section {
                    name: "Commands".into(),
                    aside: format!("{} commands", commands.len()),
                    details: commands,
                    keys: false,
                },
                Section {
                    name: "Keys".into(),
                    aside: format!("{count} keys"),
                    details: keys,
                    keys: true,
                },
            ],
            section: 0,
            offset: 0,
        }
    }

    pub fn context(mut self, context: &str) -> Self {
        context.clone_into(&mut self.context);
        self
    }

    pub fn aside(mut self, aside: &str) -> Self {
        aside.clone_into(&mut self.sections[0].aside);
        self
    }

    pub fn current(&self) -> &Section {
        &self.sections[self.section.min(self.sections.len() - 1)]
    }
}
