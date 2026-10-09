use super::{
    App, Kind,
    jobs::{CatalogPurpose, Job, ResultValue},
    selector::{Choice, Purpose, Selector, Setting},
};
use crate::{effort::Effort, openrouter::OpenRouter};

impl App {
    pub(super) fn choose(&mut self, choice: Choice) -> Result<(), String> {
        let menu = self.state.selector.take().unwrap();
        match (choice, &menu.purpose) {
            (Choice::Copy(index), Purpose::Copy) => self.choose_copy(index),
            (Choice::Draft(index), Purpose::Drafts { .. }) => {
                let discard = menu.delete_target() == Some(index);
                self.state.selector = Some(menu);
                self.choose_draft(index, discard);
            }
            (Choice::Session(index), Purpose::Sessions { current, .. }) => {
                let id = self.saved_sessions[index].id.clone();
                if menu.delete_target() == Some(index) {
                    self.state.selector = Some(menu);
                    self.start_delete(&id)?;
                } else if Some(index) != *current {
                    self.resume_session(&id);
                }
            }
            (Choice::Model(index), Purpose::Models { defaults }) => {
                let model = self.catalog[index].clone();
                let current = if *defaults {
                    self.config.settings.effort
                } else {
                    self.agent.as_ref().unwrap().effort()
                };
                self.state.selector = Some(Selector::efforts(model, *defaults, current));
            }
            (Choice::Effort(effort), Purpose::Efforts { model, defaults }) => {
                self.apply_selection(model.id.clone(), effort, *defaults)
            }
            (Choice::Setting(setting), Purpose::Settings) => {
                self.settings_parent = match setting {
                    Setting::Close => None,
                    _ => Some(setting),
                };
                match setting {
                    Setting::Model => self.load_catalog(CatalogPurpose::Models {
                        defaults: true,
                        id: None,
                    }),
                    Setting::Effort => self.load_catalog(CatalogPurpose::Effort {
                        defaults: true,
                        model: self.config.settings.model.clone(),
                        effort: None,
                    }),
                    Setting::Key => self.state.selector = Some(Selector::key()),
                    Setting::Close => {
                        self.local_finish_quiet(Kind::Notice, "Settings closed", vec![])
                    }
                }
            }
            _ => {
                self.state.selector = Some(menu);
            }
        }
        Ok(())
    }

    fn finish_setting(&mut self, kind: Kind, result: &str, details: Vec<(String, String)>) {
        if self.settings_parent.is_some() && self.pending_command.is_none() {
            self.local_start("/settings");
        }
        self.local_finish(kind, result, details);
        self.return_to_settings();
    }

    fn return_to_settings(&mut self) {
        if let Some(parent) = self.settings_parent.take() {
            let mut menu =
                Selector::settings(&self.config.settings.model, self.config.settings.effort);
            menu.selected = match parent {
                Setting::Model => 0,
                Setting::Effort => 1,
                Setting::Key => 2,
                Setting::Close => 3,
            };
            self.state.selector = Some(menu);
        }
    }

    pub(super) fn cancel_setting(&mut self) {
        self.local_finish_quiet(
            Kind::Warning,
            "Selection cancelled · previous settings kept",
            vec![],
        );
        self.return_to_settings();
    }

    pub(super) fn apply_selection(&mut self, model: String, effort: Effort, defaults: bool) {
        self.state.selector = None;
        if defaults {
            let mut settings = self.config.settings.clone();
            settings.model = model.clone();
            settings.effort = effort;
            match self.config.store.save(&settings) {
                Ok(()) => {
                    let previous = format!(
                        "{} · {}",
                        self.config.settings.model,
                        self.config.settings.effort.name()
                    );
                    self.config.settings = settings;
                    self.finish_setting(
                        Kind::Notice,
                        &format!(
                            "Defaults saved: {model} · {} · was {previous}",
                            effort.name()
                        ),
                        vec![],
                    );
                }
                Err(error) => self.finish_setting(Kind::Error, &error, vec![]),
            }
        } else {
            let agent = self.agent.as_mut().unwrap();
            let previous = format!("{} · {}", agent.model(), agent.effort().name());
            match agent.set_model(model.clone()) {
                Ok(()) => {
                    agent.set_effort(effort);
                    self.archive = agent.archive();
                    self.state.effort = effort.name().into();
                    self.finish_setting(
                        Kind::Notice,
                        &format!("Model set to {model} · {} · was {previous}", effort.name()),
                        vec![],
                    );
                }
                Err(error) => self.finish_setting(Kind::Error, &error, vec![]),
            }
        }
    }

    pub(super) fn poll_job(&mut self) -> Result<bool, String> {
        if !self.job.as_ref().is_some_and(Job::finished) {
            return Ok(false);
        }
        let result = self.job.take().unwrap().finish()?;
        self.state.selector = None;
        match result {
            ResultValue::Catalog(purpose, result) => {
                match result {
                    Err(error) => self.finish_setting(Kind::Error, &error, vec![]),
                    Ok(models) => {
                        self.catalog = models;
                        self.state.clear_progress();
                        match purpose {
                            CatalogPurpose::Models { defaults, id: None } => {
                                if self.catalog.is_empty() {
                                    self.finish_setting(
                                        Kind::Error,
                                        "No tool-capable models available",
                                        vec![],
                                    );
                                } else {
                                    self.state.selector = Some(Selector::models(
                                        &self.catalog,
                                        defaults,
                                        if defaults {
                                            &self.config.settings.model
                                        } else {
                                            &self.archive.model
                                        },
                                    ));
                                }
                            }
                            CatalogPurpose::Models {
                                defaults,
                                id: Some(id),
                            } => {
                                if let Some(model) =
                                    self.catalog.iter().find(|model| model.id == id).cloned()
                                {
                                    self.state.selector =
                                        Some(Selector::efforts(model, defaults, Effort::Default));
                                } else {
                                    self.finish_setting(
                                        Kind::Error,
                                        &format!("Model not found or does not support tools: {id}"),
                                        vec![],
                                    );
                                }
                            }
                            CatalogPurpose::Effort {
                                defaults,
                                model,
                                effort,
                            } => {
                                if let Some(model) =
                                    self.catalog.iter().find(|value| value.id == model).cloned()
                                {
                                    if let Some(effort) = effort {
                                        if model.efforts.contains(&effort) {
                                            self.apply_selection(model.id, effort, defaults);
                                        } else {
                                            self.finish_setting(
                                                Kind::Error,
                                                &format!(
                                                    "{} does not support effort {}",
                                                    model.id,
                                                    effort.name()
                                                ),
                                                vec![],
                                            );
                                        }
                                    } else {
                                        let current = if defaults {
                                            self.config.settings.effort
                                        } else {
                                            self.agent.as_ref().unwrap().effort()
                                        };
                                        self.state.selector =
                                            Some(Selector::efforts(model, defaults, current));
                                    }
                                } else {
                                    self.finish_setting(Kind::Error, "Current model is not available in the tool-capable catalog", vec![]);
                                }
                            }
                        }
                    }
                }
            }
            ResultValue::Key(key, result) => match result {
                Err(error) => self.finish_setting(Kind::Error, &error, vec![]),
                Ok(()) => {
                    let mut settings = self.config.settings.clone();
                    settings.api_key = key;
                    match self.config.store.save(&settings) {
                        Err(error) => self.finish_setting(Kind::Error, &error, vec![]),
                        Ok(()) => {
                            let agent = self.agent.as_mut().unwrap();
                            let mut client = OpenRouter::with_api(
                                agent.api().with_key(settings.api_key.clone())?,
                                agent.model().into(),
                            )?;
                            client.set_effort(agent.effort());
                            agent.replace_client(client);
                            self.config.settings = settings;
                            self.archive = agent.archive();
                            self.catalog.clear();
                            self.finish_setting(
                                Kind::Notice,
                                "OpenRouter key validated and saved",
                                vec![],
                            );
                        }
                    }
                }
            },
        }
        Ok(true)
    }

    pub(super) fn save_key(&mut self) {
        let key = self
            .state
            .selector
            .as_ref()
            .unwrap()
            .editor
            .text
            .trim()
            .to_string();
        let api = self.agent.as_ref().unwrap().api().with_key(key.clone());
        match api {
            Err(error) => self.warn(&error),
            Ok(api) => {
                self.state.clear_notice();
                let mut menu = Selector::loading();
                menu.title = "Checking OpenRouter key…".into();
                self.state.selector = Some(menu);
                self.job = Some(Job::key(api, key));
            }
        }
    }
}
