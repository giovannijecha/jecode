//! Pure, bounded command and selection surfaces inside the composer.
#[cfg(test)]
use super::{
    model::Model,
    style::{Row, Tone},
    tool_view::clipped,
};
use crate::{
    providers::openai_account::catalog::{Catalog, Entry as CatalogEntry},
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
    Effort,
    Clear,
    Status,
    DefaultModels,
    SelectModel(String, bool),
    Settings,
    Model(session::Model),
    Preference(Change),
    Context,
    Compact,
    DiscardPendingImages,
    Help,
}
#[derive(Clone)]
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
#[derive(Clone)]
pub(super) struct Panel {
    pub title: &'static str,
    pub entries: Vec<Entry>,
}
#[derive(Clone, Default)]
pub(super) struct Menu {
    pub panel: Option<Panel>,
    pub query: String,
    pub selected: usize,
    pub hidden: bool,
    pub pasted_literal: bool,
}
impl Menu {
    pub fn active(&self, text: &str) -> bool {
        self.panel.is_some()
            || text.starts_with('/') && !text.contains('\n') && !self.hidden && !self.pasted_literal
    }
    pub fn entries(&self, text: &str) -> Vec<Entry> {
        let source = self.panel.as_ref().map_or_else(commands, |panel| {
            panel
                .entries
                .iter()
                .map(|e| Entry::new(&e.label, &e.description, e.action.clone()))
                .collect()
        });
        let query = if self.panel.is_some() {
            self.query.as_str()
        } else {
            text.trim_start_matches('/').trim()
        };
        let choices: Vec<_> = source
            .iter()
            .map(|entry| super::lab::picker::Choice {
                label: entry.label.trim_start_matches('/').into(),
                detail: entry.description.clone(),
            })
            .collect();
        super::lab::picker::matches(&choices, query)
            .into_iter()
            .map(|found| source[found.index].clone())
            .collect()
    }
    pub fn close(&mut self) {
        self.panel = None;
        self.query.clear();
        self.selected = 0;
        self.hidden = true;
    }
    pub fn open(&mut self, panel: Panel) {
        self.panel = Some(panel);
        self.query.clear();
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
        (
            "/effort",
            "Set reasoning effort for this conversation",
            Effort,
        ),
        ("/status", "Show this conversation's session facts", Status),
        (
            "/clear",
            "Start fresh context and keep the visible transcript",
            Clear,
        ),
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
        (
            "/discard-pending-images",
            "Stop sending pending pixels; keep saved image evidence",
            DiscardPendingImages,
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

pub(super) fn models(catalog: &Catalog, current: session::Model, defaults: bool) -> Panel {
    Panel {
        title: if defaults { "Default model" } else { "Model" },
        entries: catalog
            .entries
            .iter()
            .filter(|entry| entry.visible && entry.compatible)
            .map(|entry| {
                Entry::new(
                    &format!(
                        "{}{}",
                        entry.id,
                        if entry.id == current.id() {
                            " · current"
                        } else {
                            ""
                        }
                    ),
                    if entry.name == entry.id {
                        ""
                    } else {
                        &entry.name
                    },
                    Action::SelectModel(entry.id.clone(), defaults),
                )
            })
            .collect(),
    }
}
pub(super) fn efforts(entry: &CatalogEntry, current: session::Model, defaults: bool) -> Panel {
    let mut entries = Vec::new();
    let option = |effort: Option<&str>, label: String| {
        let selection =
            session::Model::new(&entry.id, effort).expect("catalog identifiers validated");
        Entry::new(
            &format!(
                "{}{}",
                label,
                if selection == current {
                    " · current"
                } else {
                    ""
                }
            ),
            "",
            if defaults {
                Action::Preference(Change::Model(selection))
            } else {
                Action::Model(selection)
            },
        )
    };
    let default = entry
        .default_effort
        .as_deref()
        .map_or("unknown", |value| value);
    entries.push(option(None, format!("Provider default · {default}")));
    if let Some(efforts) = &entry.efforts {
        entries.extend(
            efforts
                .iter()
                .map(|effort| option(Some(effort), effort.clone())),
        );
    }
    Panel {
        title: "Reasoning effort",
        entries,
    }
}
pub(super) fn settings(settings: &Settings) -> Panel {
    Panel {
        title: "Settings",
        entries: vec![
            Entry::new(
                &format!(
                    "Default · {} · {}",
                    settings.model.id(),
                    settings.model.effort().unwrap_or("provider default")
                ),
                "Applies to new conversations",
                Action::DefaultModels,
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

#[cfg(test)]
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
