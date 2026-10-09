use super::editor::Editor;
use crate::{effort::Effort, openrouter::Model, session::commands::match_text};

#[derive(Clone, Copy)]
pub enum Setting {
    Model,
    Effort,
    Key,
    Close,
}
#[derive(Clone, Copy)]
pub enum Choice {
    Model(usize),
    Effort(Effort),
    Setting(Setting),
    Session(usize),
    Copy(usize),
    Draft(usize),
}
pub struct OptionRow {
    pub name: String,
    pub description: String,
    pub choice: Choice,
}
pub struct Hit {
    pub index: usize,
    pub name: Vec<usize>,
    pub description: Vec<usize>,
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
    Key,
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
    pub purpose: Purpose,
    pub options: Vec<OptionRow>,
    pub filtered: Vec<Hit>,
    pub selected: usize,
    pub editor: Editor,
    pub searchable: bool,
}

impl Selector {
    fn new(title: &str, purpose: Purpose, options: Vec<OptionRow>) -> Self {
        let searchable = options.len() > 8;
        let mut selector = Self {
            title: title.into(),
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
    pub fn loading() -> Self {
        Self::new("Loading model catalog…", Purpose::Loading, vec![])
    }
    pub fn copy(targets: &[crate::copy::Target]) -> Self {
        Self::new(
            "Copy to clipboard",
            Purpose::Copy,
            targets
                .iter()
                .enumerate()
                .map(|(index, target)| OptionRow {
                    name: super::text::clip(&target.name, 64),
                    description: crate::copy::preview(&target.text),
                    choice: Choice::Copy(index),
                })
                .collect(),
        )
    }
    pub fn drafts(queue: &super::drafts::Queue) -> Self {
        let options = queue
            .messages
            .iter()
            .map(|text| (text.as_str(), "Queued"))
            .chain(
                queue
                    .paused
                    .iter()
                    .map(|draft| (draft.text.as_str(), "Paused")),
            )
            .enumerate()
            .map(|(index, (text, status))| OptionRow {
                name: crate::copy::preview(text),
                description: status.into(),
                choice: Choice::Draft(index),
            })
            .collect();
        Self::new("Drafts", Purpose::Drafts { delete: None }, options)
    }
    pub fn sessions(sessions: &[crate::sessions::Summary], current: &str) -> Self {
        let mut menu = Self::new(
            "Resume · current folder",
            Purpose::Sessions {
                current: sessions.iter().position(|session| session.id == current),
                delete: None,
                working: false,
            },
            sessions
                .iter()
                .enumerate()
                .map(|(index, session)| OptionRow {
                    name: session.title.clone(),
                    description: format!(
                        "{}{}",
                        if session.id == current {
                            "current · "
                        } else {
                            ""
                        },
                        session.description()
                    ),
                    choice: Choice::Session(index),
                })
                .collect(),
        );
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
                name: model.id.clone(),
                description: format!(
                    "{}{}",
                    model.name,
                    if model.id == current {
                        " · current"
                    } else {
                        ""
                    }
                ),
                choice: Choice::Model(index),
            })
            .collect();
        Self::new(
            if defaults { "Default model" } else { "Model" },
            Purpose::Models { defaults },
            options,
        )
    }
    pub fn efforts(model: Model, defaults: bool, current: Effort) -> Self {
        let options = model
            .efforts
            .iter()
            .map(|effort| OptionRow {
                name: effort.name().into(),
                description: if *effort == current {
                    "Current selection".into()
                } else if *effort == Effort::Default {
                    "Use the model's default reasoning settings".into()
                } else {
                    format!("{} reasoning effort", effort.name())
                },
                choice: Choice::Effort(*effort),
            })
            .collect();
        let title = format!(
            "{} · {}",
            if defaults { "Default effort" } else { "Effort" },
            model.id
        );
        let mut menu = Self::new(&title, Purpose::Efforts { model, defaults }, options);
        menu.selected = menu
            .options
            .iter()
            .position(|option| matches!(option.choice, Choice::Effort(effort) if effort == current))
            .unwrap_or(0);
        menu
    }
    pub fn settings(model: &str, effort: Effort) -> Self {
        Self::new(
            "Settings · saved defaults",
            Purpose::Settings,
            vec![
                OptionRow {
                    name: "Default model".into(),
                    description: model.into(),
                    choice: Choice::Setting(Setting::Model),
                },
                OptionRow {
                    name: "Default effort".into(),
                    description: effort.name().into(),
                    choice: Choice::Setting(Setting::Effort),
                },
                OptionRow {
                    name: "OpenRouter key".into(),
                    description: "Configured · replace key (hidden input)".into(),
                    choice: Choice::Setting(Setting::Key),
                },
                OptionRow {
                    name: "Close".into(),
                    description: "Return to the draft".into(),
                    choice: Choice::Setting(Setting::Close),
                },
            ],
        )
    }
    pub fn key() -> Self {
        Self::new("OpenRouter key", Purpose::Key, vec![])
    }
    pub fn refresh(&mut self) {
        self.cancel_delete();
        let mut filtered: Vec<_> = self
            .options
            .iter()
            .enumerate()
            .filter_map(|(index, option)| {
                let name = match_text(&self.editor.text, &option.name);
                let description = match_text(&self.editor.text, &option.description);
                match (name, description) {
                    (Some((score, positions)), other)
                        if other.as_ref().is_none_or(|(other, _)| score <= *other) =>
                    {
                        Some((
                            score,
                            Hit {
                                index,
                                name: positions,
                                description: vec![],
                            },
                        ))
                    }
                    (_, Some((score, positions))) => Some((
                        score,
                        Hit {
                            index,
                            name: vec![],
                            description: positions,
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
