//! Pure, bounded command and selection surfaces above the composer.
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
    Quit,
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
        ("/quit", "Save and exit", Quit),
    ]
    .into_iter()
    .map(|(name, description, action)| Entry::new(name, description, action))
    .collect()
}

pub(super) fn models(current: session::Model) -> Panel {
    Panel {
        title: "Model · this conversation",
        entries: [session::Model::Luna, session::Model::Terra]
            .into_iter()
            .map(|model| {
                Entry::new(
                    model.id(),
                    if model == current {
                        "Current · medium effort"
                    } else {
                        "Medium effort · keeps this conversation"
                    },
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
        title: "Settings · saved in ~/.jecode/v1/settings.json",
        entries: vec![
            Entry::new(
                &format!("Default model · {}", settings.model.id()),
                &format!("New conversations · Enter selects {}", next.id()),
                Action::Preference(Change::Model(next)),
            ),
            Entry::new(
                &format!("File access · {}", settings.file_access.name()),
                "New conversations · Enter switches local/workspace",
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
                "Enter toggles · applies now and on next launch",
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
        title: "Resume · type to filter by title or folder",
        entries: sessions
            .into_iter()
            .filter(|s| s.model.is_some() && Some(s.id.as_str()) != current)
            .map(|s| {
                let path = s
                    .workspace
                    .as_ref()
                    .map_or("Conversation only".into(), |p| p.to_string_lossy());
                let path = path.strip_prefix(r"\\?\").unwrap_or(&path);
                Entry::new(
                    if s.title.trim().is_empty() {
                        "Untitled conversation"
                    } else {
                        &s.title
                    },
                    &format!("{path} · {} turns", s.turns),
                    Action::Resume(s.id),
                )
            })
            .collect(),
    }
}

pub(super) fn rows(model: &Model, width: usize, available: usize) -> Vec<Row> {
    if model.account.is_none() || !model.menu.active(&model.editor.text) || available < 2 {
        return Vec::new();
    }
    let menu = &model.menu;
    let entries = menu.entries(&model.editor.text);
    let title = menu.panel.as_ref().map_or("Commands", |p| p.title);
    let mut rows = vec![clipped(title, width, Tone::Heading)];
    if entries.is_empty() {
        rows.push(clipped("No matches · Esc closes", width, Tone::Muted));
        return rows;
    }
    let selected = menu.selected.min(entries.len() - 1);
    let count = available.saturating_sub(3).clamp(1, 6).min(entries.len());
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
                Tone::Muted
            },
        ));
    }
    if rows.len() < available {
        rows.push(clipped(&entries[selected].description, width, Tone::Muted));
    }
    if rows.len() < available {
        rows.push(clipped(
            &format!(
                "↑↓ choose · Enter select · Esc close   {}/{}",
                selected + 1,
                entries.len()
            ),
            width,
            Tone::Muted,
        ));
    }
    rows.truncate(available);
    rows
}
