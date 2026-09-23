//! Pure, bounded command and selection surfaces inside the composer.
use super::{
    model::Model,
    style::{Row, Tone},
    tool_view::clipped,
};
use crate::{
    session,
    state::settings::{Change, Settings},
};

#[derive(Clone)]
pub(super) enum Action {
    Login,
    Logout,
    New,
    Browse,
    Resume(String),
    Models,
    Settings,
    Model(session::Model),
    Preference(Change),
    Context,
    Compact,
    Help,
}
pub(super) struct Entry {
    pub label: String,
    pub description: String,
    pub action: Action,
}
impl Entry {
    fn new(label: &str, description: &str, action: Action) -> Self {
        Self {
            label: label.into(),
            description: description.into(),
            action,
        }
    }
}
pub(super) struct Panel {
    pub title: &'static str,
    pub entries: Vec<Entry>,
}
#[derive(Default)]
pub(super) struct Menu {
    pub panel: Option<Panel>,
    pub selected: usize,
    pub hidden: bool,
}
impl Menu {
    pub fn active(&self, text: &str) -> bool {
        self.panel.is_some() || text.starts_with('/') && !self.hidden
    }
    pub fn entries(&self, text: &str) -> Vec<Entry> {
        let source = self.panel.as_ref().map_or_else(commands, |panel| {
            panel
                .entries
                .iter()
                .map(|e| Entry::new(&e.label, &e.description, e.action.clone()))
                .collect()
        });
        let query = text.trim_start_matches('/').trim().to_lowercase();
        source
            .into_iter()
            .filter(|e| {
                if self.panel.is_none() {
                    e.label.trim_start_matches('/').starts_with(&query)
                } else {
                    format!("{} {}", e.label, e.description)
                        .to_lowercase()
                        .contains(&query)
                }
            })
            .collect()
    }
    pub fn close(&mut self) {
        self.panel = None;
        self.selected = 0;
        self.hidden = true;
    }
    pub fn open(&mut self, panel: Panel) {
        self.panel = Some(panel);
        self.selected = 0;
        self.hidden = false;
    }
}

pub(super) fn commands() -> Vec<Entry> {
    use Action::*;
    [
        ("/new", "Start a new conversation in this directory", New),
        ("/resume", "Find and reopen a saved conversation", Browse),
        ("/model", "Choose the model for this conversation", Models),
        ("/settings", "Change saved defaults and animation", Settings),
        (
            "/context",
            "Inspect measured context and token usage",
            Context,
        ),
        (
            "/compact",
            "Summarize earlier context; retain full history",
            Compact,
        ),
        ("/help", "Show shortcuts and available commands", Help),
        ("/login", "Sign in to the saved account", Login),
        (
            "/logout",
            "Remove local account access; keep this conversation",
            Logout,
        ),
    ]
    .into_iter()
    .map(|(name, description, action)| Entry::new(name, description, action))
    .collect()
}

pub(super) fn models(current: session::Model) -> Panel {
    Panel {
        title: "Model",
        entries: [session::Model::Luna, session::Model::Terra]
            .into_iter()
            .map(|model| {
                Entry::new(
                    &format!(
                        "{}{}",
                        model.id(),
                        if model == current { " · current" } else { "" }
                    ),
                    "",
                    Action::Model(model),
                )
            })
            .collect(),
    }
}
pub(super) fn settings(settings: &Settings) -> Panel {
    let next = if settings.model == session::Model::Luna {
        session::Model::Terra
    } else {
        session::Model::Luna
    };
    Panel {
        title: "Settings",
        entries: vec![
            Entry::new(
                &format!("Default model · {}", settings.model.id()),
                "Applies to new conversations",
                Action::Preference(Change::Model(next)),
            ),
            Entry::new(
                &format!("File access · {}", settings.file_access.name()),
                "Applies to new conversations",
                Action::Preference(Change::ToggleAccess),
            ),
            Entry::new(
                &format!(
                    "Animation · {}",
                    if settings.reduced_motion {
                        "reduced"
                    } else {
                        "on"
                    }
                ),
                "Applies immediately",
                Action::Preference(Change::ToggleMotion),
            ),
        ],
    }
}
pub(super) fn sessions(
    sessions: Vec<session::persistence::Listed>,
    current: Option<&str>,
) -> Panel {
    Panel {
        title: "Resume",
        entries: sessions
            .into_iter()
            .filter(|s| s.model.is_some() && Some(s.id.as_str()) != current)
            .map(|s| {
                let path = s
                    .directory
                    .as_ref()
                    .map_or("Conversation only".into(), |p| p.to_string_lossy());
                let path = path.strip_prefix(r"\\?\").unwrap_or(&path);
                Entry::new(
                    if s.title.trim().is_empty() {
                        "Untitled conversation"
                    } else {
                        &s.title
                    },
                    &format!(
                        "{path} · {} turns{}",
                        s.turns,
                        if s.workspace.is_none() {
                            " · Conversation only"
                        } else {
                            ""
                        }
                    ),
                    Action::Resume(s.id),
                )
            })
            .collect(),
    }
}

pub(super) fn rows(model: &Model, width: usize, available: usize) -> Vec<Row> {
    if model.account.is_none() || !model.menu.active(&model.editor.text) || available == 0 {
        return Vec::new();
    }
    let menu = &model.menu;
    let entries = menu.entries(&model.editor.text);
    let mut rows = Vec::new();
    if let Some(panel) = &menu.panel {
        rows.push(clipped(panel.title, width, Tone::Heading));
        if available == 1 {
            return rows;
        }
    }
    if entries.is_empty() {
        rows.push(clipped("No matches", width, Tone::Muted));
        return rows;
    }
    let selected = menu.selected.min(entries.len() - 1);
    let details = menu.panel.is_some() && !entries[selected].description.is_empty();
    let count = available
        .saturating_sub(rows.len() + usize::from(details))
        .max(1)
        .min(entries.len());
    let start = selected.saturating_sub(count - 1);
    for (index, entry) in entries.iter().enumerate().skip(start).take(count) {
        rows.push(clipped(
            &format!(
                "{} {}",
                if index == selected { "›" } else { " " },
                entry.label
            ),
            width,
            if index == selected {
                Tone::Accent
            } else {
                Tone::Text
            },
        ));
    }
    if details && rows.len() < available {
        rows.push(clipped(&entries[selected].description, width, Tone::Muted));
    }
    rows.truncate(available);
    rows
}
