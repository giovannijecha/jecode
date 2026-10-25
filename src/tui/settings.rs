use super::{
    App, Kind,
    jobs::{CatalogPurpose, Job, ResultValue},
    selector::{Choice, Purpose, Selector, Setting},
};
use crate::{backend, effort::Effort};

impl App {
    pub(super) fn choose(&mut self, choice: Choice) -> Result<(), String> {
        let menu = self.state.selector.take().unwrap();
        match (choice, &menu.purpose) {
            (Choice::Copy(index), Purpose::Copy) => self.choose_copy(index),
            (Choice::Draft(slot), Purpose::Drafts { .. }) => {
                let discard = menu.delete_target().is_some_and(|target| {
                    matches!(menu.options[target].choice, Choice::Draft(marked) if marked == slot)
                });
                self.state.selector = Some(menu);
                self.choose_draft(slot, discard);
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
                self.show_efforts(model, *defaults, current);
            }
            (Choice::Provider(id), Purpose::Providers { defaults }) => {
                self.provider_parent = Some(*defaults);
                let home = self
                    .config
                    .store
                    .path()
                    .parent()
                    .expect("configuration directory");
                let configured = self.config.settings.services(home, id);
                let services = if !defaults
                    && self.agent.as_ref().unwrap().backend_id() == id
                    && configured
                        .as_ref()
                        .ok()
                        .and_then(|service| service.account())
                        == self.agent.as_ref().unwrap().services().account()
                {
                    Ok(self.agent.as_ref().unwrap().services())
                } else {
                    configured
                };
                match services {
                    Ok(services) => self.load_catalog_for(
                        CatalogPurpose::Models {
                            defaults: *defaults,
                            id: None,
                        },
                        services,
                    ),
                    Err(error) => self.finish_setting(Kind::Error, &error, vec![]),
                }
            }
            (Choice::Credentials, Purpose::ProviderSettings) => self.open_credentials(),
            (Choice::ApiKeys, Purpose::ProviderSettings) => self.open_api_keys(),
            (Choice::Provider("openai"), Purpose::Credentials) => self.open_accounts(),
            (Choice::OpenRouterKey, Purpose::ApiKeys) => {
                self.state.selector = Some(Selector::key())
            }
            (Choice::Account(index), Purpose::Accounts) => self.open_account(index),
            (Choice::Account(_), Purpose::Account { .. }) => self.activate_account(),
            (Choice::AddAccount, Purpose::Accounts) => self.start_sign_in(None),
            (
                Choice::SignOut,
                Purpose::Account {
                    confirm_sign_out: false,
                },
            ) => {
                self.state.selector = Some(menu);
                self.confirm_sign_out();
            }
            (
                Choice::SignOut,
                Purpose::Account {
                    confirm_sign_out: true,
                },
            ) => self.start_sign_out(),
            (Choice::ManageUsage, Purpose::Account { .. }) => {
                self.state.selector = Some(menu);
                self.manage_usage();
            }
            (Choice::Effort(effort), Purpose::Efforts { model, defaults }) => {
                self.apply_selection(model.id.clone(), effort, *defaults)
            }
            (Choice::Routing(index), Purpose::Routing { .. }) => self.choose_routing(menu, index),
            (Choice::Setting(setting), Purpose::Settings) => {
                self.settings_parent = Some(setting);
                match setting {
                    Setting::Model => self.open_model_providers(true),
                    Setting::Effort => self.load_catalog(CatalogPurpose::Effort {
                        defaults: true,
                        model: self.config.settings.model.clone(),
                        effort: None,
                    }),
                    Setting::Routing => self.open_routing(true),
                    Setting::AutoCompact => {
                        self.state.selector =
                            Some(Selector::auto_compact(self.config.settings.auto_compact))
                    }
                    Setting::Providers => {
                        self.state.selector = Some(Selector::providers_settings())
                    }
                }
            }
            _ => {
                self.state.selector = Some(menu);
            }
        }
        Ok(())
    }

    pub(super) fn finish_setting(
        &mut self,
        kind: Kind,
        result: &str,
        details: Vec<(String, String)>,
    ) {
        self.finish_setting_with_change(kind, result, details, false);
    }

    pub(super) fn finish_setting_change(
        &mut self,
        kind: Kind,
        result: &str,
        details: Vec<(String, String)>,
    ) {
        self.finish_setting_with_change(kind, result, details, true);
    }

    fn finish_setting_with_change(
        &mut self,
        kind: Kind,
        result: &str,
        details: Vec<(String, String)>,
        change: bool,
    ) {
        if self.settings_parent.is_some() && self.pending_command.is_none() {
            self.local_start("/settings");
        }
        if change {
            self.local_finish_change(kind, result, details);
        } else {
            self.local_finish(kind, result, details);
        }
        self.return_to_settings();
    }

    pub(super) fn return_to_settings(&mut self) {
        if let Some(parent) = self.settings_parent.take() {
            let mut menu = self.settings_menu();
            menu.selected = menu.setting_index(parent);
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
        self.auth_model_switch = false;
        let selected = self.catalog_services.take();
        self.provider_parent = None;
        if defaults {
            let mut settings = self.config.settings.clone();
            if let Some(services) = &selected
                && services.id() != settings.backend_id
            {
                let key = settings
                    .credential_for(services.id())
                    .unwrap_or("")
                    .to_owned();
                if let Err(error) = settings.select_backend(services.id(), key) {
                    self.finish_setting(Kind::Error, &error, vec![]);
                    return;
                }
            }
            settings.model = model.clone();
            settings.effort = effort;
            match self.config.store.save(&settings) {
                Ok(()) => {
                    if let Some(services) = selected {
                        self.agent.as_mut().unwrap().register_backend(services);
                        self.archive = self.agent.as_ref().unwrap().archive();
                    }
                    let changed_provider = self.config.settings.backend_id != settings.backend_id;
                    let previous = format!(
                        "{} ({})",
                        if changed_provider {
                            format!(
                                "{} / {}",
                                self.config.settings.backend_id, self.config.settings.model
                            )
                        } else {
                            self.config.settings.model.clone()
                        },
                        self.config.settings.effort.name()
                    );
                    self.config.settings = settings;
                    let next = if changed_provider {
                        format!("{} / {model}", self.config.settings.backend_id)
                    } else {
                        model.clone()
                    };
                    self.finish_setting_change(
                        Kind::Notice,
                        &format!(
                            "Default model: {previous} → {next} ({}) · new conversations",
                            effort.name()
                        ),
                        vec![],
                    );
                }
                Err(error) => self.finish_setting(Kind::Error, &error, vec![]),
            }
        } else {
            let agent = self.agent.as_mut().unwrap();
            let previous_provider = agent.backend_id();
            let previous_model = agent.model().to_owned();
            let previous_effort = agent.effort();
            let switch = selected.as_ref().is_some_and(|services| {
                agent.needs_configuration()
                    || services.id() != agent.backend_id()
                    || services.account() != agent.services().account()
            });
            let result = if switch {
                selected
                    .unwrap()
                    .create(model.clone())
                    .and_then(|mut client| {
                        client.set_effort(effort);
                        agent.replace_backend(client)
                    })
            } else {
                agent.set_model(model.clone())
            };
            match result {
                Ok(()) => {
                    agent.set_effort(effort);
                    let changed_provider = agent.backend_id() != previous_provider;
                    let previous = if changed_provider {
                        format!(
                            "{previous_provider} / {previous_model} ({})",
                            previous_effort.name()
                        )
                    } else {
                        format!("{previous_model} ({})", previous_effort.name())
                    };
                    let next = if changed_provider {
                        format!("{} / {model}", agent.backend_id())
                    } else {
                        model.clone()
                    };
                    self.archive = agent.archive();
                    self.state.effort = effort.name().into();
                    self.refresh_context();
                    self.finish_setting_change(
                        Kind::Notice,
                        &format!(
                            "Model: {previous} → {next} ({}) · this conversation",
                            effort.name()
                        ),
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
                    Err(error) => match purpose {
                        CatalogPurpose::Models {
                            defaults,
                            id: Some(id),
                        } => {
                            self.apply_selection(id, Effort::Default, defaults);
                        }
                        _ if self.auth_model_switch => {
                            self.auth_model_switch = false;
                            self.open_accounts();
                            self.warn(&error);
                        }
                        _ => self.finish_setting(Kind::Error, &error, vec![]),
                    },
                    Ok(models) => {
                        self.catalog = models;
                        self.state.clear_progress();
                        match purpose {
                            CatalogPurpose::Models { defaults, id: None } => {
                                if self.catalog.is_empty() {
                                    if self.auth_model_switch {
                                        self.auth_model_switch = false;
                                        self.open_accounts();
                                        self.warn("No tool-capable models available");
                                    } else {
                                        self.finish_setting(
                                            Kind::Error,
                                            "No tool-capable models available",
                                            vec![],
                                        );
                                    }
                                } else {
                                    self.state.selector = Some(self.models_menu(defaults));
                                }
                            }
                            CatalogPurpose::Models {
                                defaults,
                                id: Some(id),
                            } => {
                                if let Some(model) =
                                    self.catalog.iter().find(|model| model.id == id).cloned()
                                {
                                    self.show_efforts(model, defaults, Effort::Default);
                                } else {
                                    self.apply_selection(id, Effort::Default, defaults);
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
                                        if model.efforts.contains(&effort)
                                            || self.catalog_services.as_ref().is_some_and(
                                                |services| !services.validates_effort(),
                                            )
                                        {
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
                                        self.show_efforts(model, defaults, current);
                                    }
                                } else {
                                    self.finish_setting(Kind::Error, "Current model is not available in the tool-capable catalog", vec![]);
                                }
                            }
                        }
                    }
                }
            }
            ResultValue::Endpoints(purpose, result) => self.show_routing(purpose, result),
            ResultValue::SignIn(result) => self.finish_sign_in(result),
            ResultValue::SignOut(result) => self.finish_sign_out(result),
            ResultValue::Key(key, result) => self.finish_key(key, result)?,
        }
        Ok(true)
    }

    fn show_efforts(&mut self, model: backend::Model, defaults: bool, current: Effort) {
        let unknown = model.efforts == [Effort::Default];
        let mut menu = Selector::efforts(model, defaults, current);
        if self.provider_parent.is_some() {
            menu.parent = Some("Model");
        }
        if unknown
            && self
                .catalog_services
                .as_ref()
                .is_some_and(|services| !services.validates_effort())
        {
            for option in &mut menu.options {
                option.prose = "The provider chooses the level. /effort NAME requests a specific level; the provider validates it.".into();
            }
        }
        self.state.selector = Some(menu);
    }

    /// Saves the threshold, then applies it to the active conversation. An
    /// invalid value keeps the panel open; a failed save keeps both unchanged.
    pub(super) fn save_auto_compact(&mut self) {
        let text = &self.state.selector.as_ref().unwrap().editor.text;
        let text = text.trim();
        let percent = text
            .strip_suffix('%')
            .unwrap_or(text)
            .trim_end()
            .parse::<usize>()
            .map_err(|_| "Auto-compact must be a whole percentage from 1 to 99".to_string())
            .and_then(crate::context::auto_compact_percent);
        let percent = match percent {
            Ok(percent) => percent,
            Err(error) => return self.warn(&format!("{error} · previous setting kept")),
        };
        self.state.selector = None;
        let mut settings = self.config.settings.clone();
        settings.auto_compact = percent;
        match self.config.store.save(&settings) {
            Ok(()) => {
                let previous = self.config.settings.auto_compact;
                self.config.settings = settings;
                // /settings runs between turns, while the application holds the agent.
                if let Some(agent) = &mut self.agent {
                    agent.set_auto_compact(percent);
                }
                self.refresh_context();
                self.finish_setting_change(
                    Kind::Notice,
                    &format!("Auto-compact: {previous}% → {percent}% · this and new conversations"),
                    vec![],
                );
            }
            Err(error) => self.finish_setting(Kind::Error, &error, vec![]),
        }
    }
}
