use super::editor::Editor;
use crate::{
    backend::{Entry, Model, Routing},
    effort::Effort,
    session::commands::match_text,
};
use std::path::Path;
use std::time::Instant;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Setting {
    Model,
    Effort,
    Routing,
    AutoCompact,
    Key,
}
#[derive(Clone, Copy)]
pub enum Choice {
    Model(usize),
    Effort(Effort),
    Routing(usize),
    Setting(Setting),
    Session(usize),
    Copy(usize),
    Draft(super::drafts::Slot),
}
/// The right-aligned status of an option.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Badge {
    Current,
    Note(String),
    Queued,
    Paused,
}
pub struct OptionRow {
    pub name: String,
    /// One short fact beside the name, such as a model's friendly name.
    pub fact: String,
    /// Searchable detail that does not need space in the row.
    pub search: String,
    pub badge: Option<Badge>,
    /// The focused option's prose, shown in the detail strip.
    pub prose: String,
    pub choice: Choice,
}
pub struct Hit {
    pub index: usize,
    pub name: Vec<usize>,
    pub fact: Vec<usize>,
}

pub enum Purpose {
    Models {
        defaults: bool,
    },
    Efforts {
        model: Model,
        defaults: bool,
    },
    Settings,
    /// Routing for the conversation, or the saved default for `model`.
    Routing {
        defaults: bool,
        model: String,
        entries: Vec<Entry>,
        /// Muted rows after the focused choice's help.
        notes: Vec<String>,
    },
    Key,
    /// Percentage input for the automatic compaction threshold.
    AutoCompact,
    Loading,
    Sessions {
        current: Option<usize>,
        delete: Option<usize>,
        working: bool,
    },
    Copy,
    Drafts {
        delete: Option<usize>,
    },
}

pub struct Selector {
    pub title: String,
    /// Muted context after the title, such as the model a choice applies to.
    pub context: String,
    /// The menu this one returns to with Esc, shown as a breadcrumb.
    pub parent: Option<&'static str>,
    /// What a waiting panel is doing.
    pub message: String,
    pub opened: Instant,
    pub purpose: Purpose,
    pub options: Vec<OptionRow>,
    pub filtered: Vec<Hit>,
    pub selected: usize,
    pub editor: Editor,
    pub searchable: bool,
}

fn row(name: impl Into<String>, fact: impl Into<String>, choice: Choice) -> OptionRow {
    OptionRow {
        name: name.into(),
        fact: fact.into(),
        search: String::new(),
        badge: None,
        prose: String::new(),
        choice,
    }
}

const SETTINGS: &str = "Settings";

impl Selector {
    fn new(title: &str, purpose: Purpose, options: Vec<OptionRow>) -> Self {
        let searchable = options.len() > 8;
        let mut selector = Self {
            title: title.into(),
            context: String::new(),
            parent: None,
            message: String::new(),
            opened: Instant::now(),
            purpose,
            options,
            filtered: vec![],
            selected: 0,
            editor: Editor::default(),
            searchable,
        };
        selector.refresh();
        selector
    }
    fn under_settings(mut self, defaults: bool) -> Self {
        self.parent = defaults.then_some(SETTINGS);
        self
    }
    /// A panel that waits for `message`, titled after what it will show.
    pub fn loading(title: &str, message: &str, settings: bool) -> Self {
        let mut menu = Self::new(title, Purpose::Loading, vec![]).under_settings(settings);
        message.clone_into(&mut menu.message);
        menu
    }
    pub fn copy(targets: &[crate::copy::Target]) -> Self {
        let options = targets
            .iter()
            .enumerate()
            .map(|(index, target)| {
                let preview = crate::copy::preview(&target.text);
                OptionRow {
                    prose: preview.clone(),
                    search: preview,
                    ..row(super::text::clip(&target.name, 64), "", Choice::Copy(index))
                }
            })
            .collect();
        let mut menu = Self::new("Copy to clipboard", Purpose::Copy, options);
        "last response".clone_into(&mut menu.context);
        menu
    }
    pub fn drafts(queue: &super::drafts::Queue) -> Self {
        let options = queue
            .drafts()
            .into_iter()
            .map(|(slot, text)| {
                let mut name = crate::copy::preview(&text);
                if text.trim().contains('\n') && !name.ends_with('…') {
                    name.push('…');
                }
                OptionRow {
                    badge: Some(match slot {
                        super::drafts::Slot::Queued(_) => Badge::Queued,
                        super::drafts::Slot::Paused(_) => Badge::Paused,
                    }),
                    prose: text.split_whitespace().collect::<Vec<_>>().join(" "),
                    search: match slot {
                        super::drafts::Slot::Queued(_) => "Queued",
                        super::drafts::Slot::Paused(_) => "Paused",
                    }
                    .into(),
                    ..row(name, "", Choice::Draft(slot))
                }
            })
            .collect();
        Self::new("Drafts", Purpose::Drafts { delete: None }, options)
    }
    pub fn sessions(sessions: &[crate::sessions::Summary], current: &str) -> Self {
        let options = sessions
            .iter()
            .enumerate()
            .map(|(index, session)| OptionRow {
                badge: Some(if session.id == current {
                    Badge::Current
                } else {
                    Badge::Note(session.age())
                }),
                prose: session.origin(),
                search: session.description(),
                ..row(session.title.clone(), "", Choice::Session(index))
            })
            .collect();
        let mut menu = Self::new(
            "Resume",
            Purpose::Sessions {
                current: sessions.iter().position(|session| session.id == current),
                delete: None,
                working: false,
            },
            options,
        );
        "current folder".clone_into(&mut menu.context);
        menu.selected = sessions
            .iter()
            .position(|session| session.id != current)
            .unwrap_or(0);
        menu
    }
    pub fn models(models: &[Model], defaults: bool, current: &str) -> Self {
        let options = models
            .iter()
            .enumerate()
            .map(|(index, model)| OptionRow {
                badge: (model.id == current).then_some(Badge::Current),
                ..row(model.id.clone(), model.name.clone(), Choice::Model(index))
            })
            .collect();
        let mut menu = Self::new(
            if defaults { "Default model" } else { "Model" },
            Purpose::Models { defaults },
            options,
        )
        .under_settings(defaults);
        if !defaults {
            "this conversation".clone_into(&mut menu.context);
        }
        menu
    }
    pub fn efforts(model: Model, defaults: bool, current: Effort) -> Self {
        let options = model
            .efforts
            .iter()
            .map(|effort| OptionRow {
                badge: (*effort == current).then_some(Badge::Current),
                ..row(effort.name(), "", Choice::Effort(*effort))
            })
            .collect();
        let context = model.id.clone();
        let mut menu = Self::new(
            if defaults { "Default effort" } else { "Effort" },
            Purpose::Efforts { model, defaults },
            options,
        )
        .under_settings(defaults);
        menu.context = context;
        menu.selected = menu
            .options
            .iter()
            .position(|option| matches!(option.choice, Choice::Effort(effort) if effort == current))
            .unwrap_or(0);
        menu
    }
    /// `routing` is the default model's saved routing.
    pub fn settings(
        model: &str,
        effort: Effort,
        auto_compact: u8,
        routing: &Routing,
        config_path: &Path,
    ) -> Self {
        let setting = |name: &str, value: String, prose: &str, setting| OptionRow {
            prose: prose.into(),
            ..row(name, value, Choice::Setting(setting))
        };
        let mut menu = Self::new(
            SETTINGS,
            Purpose::Settings,
            vec![
                setting(
                    "Default model",
                    model.into(),
                    "Model for new conversations. /model changes only the current one.",
                    Setting::Model,
                ),
                setting(
                    "Default effort",
                    effort.name().into(),
                    "Reasoning effort for new conversations.",
                    Setting::Effort,
                ),
                setting(
                    "Default routing",
                    routing.label(),
                    "How OpenRouter picks an endpoint for the default model.",
                    Setting::Routing,
                ),
                setting(
                    "Auto-compact at",
                    format!("{auto_compact}%"),
                    "Compact when the next request reaches this share of the context window.",
                    Setting::AutoCompact,
                ),
                setting(
                    "API key",
                    "configured".into(),
                    &format!(
                        "Your OpenRouter key, kept in plain text in {}. Input stays hidden.",
                        config_path.display()
                    ),
                    Setting::Key,
                ),
            ],
        );
        "saved defaults".clone_into(&mut menu.context);
        menu
    }
    /// The settings row of `setting`, to restore focus on return.
    pub fn setting_index(&self, setting: Setting) -> usize {
        self.options
            .iter()
            .position(
                |option| matches!(option.choice, Choice::Setting(choice) if choice == setting),
            )
            .unwrap_or(0)
    }
    /// Focus starts on `current`, which the entries mark.
    pub fn routing(
        model: &str,
        defaults: bool,
        entries: Vec<Entry>,
        current: &Routing,
        notes: Vec<String>,
    ) -> Self {
        let options = entries
            .iter()
            .enumerate()
            .map(|(index, entry)| OptionRow {
                badge: (entry.routing == *current).then_some(Badge::Current),
                search: entry.description.clone(),
                // The table shows the figures; the strip keeps the other facts.
                prose: if entry.metrics.is_some() && !entry.facts.is_empty() {
                    format!("{} · {}", entry.help, entry.facts)
                } else {
                    entry.help.clone()
                },
                ..row(
                    entry.name.clone(),
                    entry.facts.clone(),
                    Choice::Routing(index),
                )
            })
            .collect();
        let focus = entries
            .iter()
            .position(|entry| entry.routing == *current)
            .unwrap_or(0);
        let purpose = Purpose::Routing {
            defaults,
            model: model.into(),
            entries,
            notes,
        };
        let mut menu = Self::new(
            if defaults {
                "Default routing"
            } else {
                "Routing"
            },
            purpose,
            options,
        )
        .under_settings(defaults);
        model.clone_into(&mut menu.context);
        menu.selected = focus;
        menu
    }
    pub fn key() -> Self {
        Self::new("API key", Purpose::Key, vec![]).under_settings(true)
    }
    pub fn auto_compact(current: u8) -> Self {
        let mut menu =
            Self::new("Auto-compact at", Purpose::AutoCompact, vec![]).under_settings(true);
        menu.editor.set(current.to_string().into());
        menu
    }
    /// Whether typed text goes to this panel's input row.
    pub fn accepts_text(&self) -> bool {
        self.searchable || matches!(self.purpose, Purpose::Key | Purpose::AutoCompact)
    }
    pub fn refresh(&mut self) {
        self.cancel_delete();
        let mut filtered: Vec<_> = self
            .options
            .iter()
            .enumerate()
            .filter_map(|(index, option)| {
                let name = match_text(&self.editor.text, &option.name);
                let fact = match_text(&self.editor.text, &option.fact);
                let search = match_text(&self.editor.text, &option.search);
                let fact = match (fact, search) {
                    (Some(fact), Some(search)) if fact.0 <= search.0 => Some(fact),
                    (_, Some((score, _))) => Some((score, vec![])),
                    (fact, None) => fact,
                };
                match (name, fact) {
                    (Some((score, positions)), other)
                        if other.as_ref().is_none_or(|(other, _)| score <= *other) =>
                    {
                        Some((
                            score,
                            Hit {
                                index,
                                name: positions,
                                fact: vec![],
                            },
                        ))
                    }
                    (_, Some((score, positions))) => Some((
                        score,
                        Hit {
                            index,
                            name: vec![],
                            fact: positions,
                        },
                    )),
                    _ => None,
                }
            })
            .collect();
        filtered.sort_by_key(|(score, hit)| (*score, hit.index));
        self.filtered = filtered.into_iter().map(|(_, hit)| hit).collect();
        self.selected = 0;
    }
    pub fn move_selection(&mut self, up: bool) {
        self.cancel_delete();
        self.selected = if up {
            self.selected.saturating_sub(1)
        } else {
            (self.selected + 1).min(self.filtered.len().saturating_sub(1))
        };
    }
    pub fn chosen(&self) -> Option<Choice> {
        self.filtered
            .get(self.selected)
            .map(|hit| self.options[hit.index].choice)
    }
    pub fn delete_target(&self) -> Option<usize> {
        match &self.purpose {
            Purpose::Sessions { delete, .. } | Purpose::Drafts { delete } => *delete,
            _ => None,
        }
    }
    pub fn cancel_delete(&mut self) -> bool {
        match &mut self.purpose {
            Purpose::Sessions { delete, .. } | Purpose::Drafts { delete } => {
                delete.take().is_some()
            }
            _ => false,
        }
    }
    pub fn toggle_delete(&mut self) {
        let index = self.filtered.get(self.selected).map(|hit| hit.index);
        if let Purpose::Sessions { delete, .. } | Purpose::Drafts { delete } = &mut self.purpose {
            *delete = if *delete == index { None } else { index };
        }
    }
}

#[cfg(test)]
mod tests;
