use super::{
    App, Kind,
    held::{self, Deferred},
    jobs::{CatalogPurpose, Job, ResultValue},
    selector::{Choice, Purpose, Selector, Setting},
};
use crate::{backend, effort::Effort};

mod auto_compact;

impl App {
    pub(super) fn choose(&mut self, choice: Choice) -> Result<(), String> {
        let menu = self.state.selector.take().unwrap();
        match (choice, &menu.purpose) {
            (Choice::Copy(index), Purpose::Copy) => self.choose_copy(index),
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
                    self.facts().effort
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
                let facts = self.facts();
                let services = if !defaults
                    && !facts.needs_configuration
                    && facts.backend_id == id
                    && configured
                        .as_ref()
                        .ok()
                        .and_then(|service| service.account())
                        == facts.services.account()
                {
                    Ok(facts.services.clone())
                } else {
                    configured
                };
                drop(facts);
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
            (Choice::Provider(provider @ ("openai" | "anthropic")), Purpose::Credentials) => {
                self.open_accounts_for(provider)
            }
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
                    Setting::Permissions => {
                        self.state.selector = Some(Selector::default_permissions(
                            self.config.settings.default_permissions,
                        ))
                    }
                    Setting::AutoCompact => {
                        self.state.selector =
                            Some(Selector::auto_compact(self.config.settings.auto_compact))
                    }
                    Setting::Providers => {
                        self.state.selector = Some(Selector::providers_settings())
                    }
                }
            }
            (
                _,
                Purpose::Permissions { .. }
                | Purpose::DefaultPermissions
                | Purpose::Remembered { .. },
            ) => self.choose_permissions(choice, menu),
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
        // Completing first-time setup saves a real selection and activates it.
        if !self.config.settings.has_model() && !defaults {
            return self.apply_selection(model, effort, true);
        }
        self.state.selector = None;
        self.auth_model_switch = false;
        let selected = self.catalog_services.take();
        self.provider_parent = None;
        if defaults {
            let first_setup = !self.config.settings.has_model();
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
                        if first_setup {
                            let result = services.create(model.clone()).and_then(|mut client| {
                                client.set_effort(effort);
                                self.agent.as_mut().unwrap().replace_backend(client)
                            });
                            if let Err(error) = result {
                                self.config.settings = settings;
                                self.finish_setting(Kind::Error, &error, vec![]);
                                return;
                            }
                            self.state.effort = effort.name().into();
                        }
                        self.defer(Deferred::Register(services));
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
                    if first_setup {
                        self.refresh_context();
                        self.finish_setting(
                            Kind::Notice,
                            &format!(
                                "Configuration saved · {model} ({}) · ready for this conversation",
                                effort.name()
                            ),
                            vec![],
                        );
                        return;
                    }
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
        } else if self.agent.is_none() {
            let facts = self.facts();
            let provider = selected
                .as_ref()
                .map(backend::Services::id)
                .filter(|id| *id != facts.backend_id);
            let notice = format!(
                "Model: {} → {} · from the next turn",
                held::selection(
                    provider.map(|_| facts.backend_id),
                    &facts.model,
                    facts.effort
                ),
                held::selection(provider, &model, effort)
            );
            drop(facts);
            self.defer(Deferred::Model {
                model,
                effort,
                services: selected,
            });
            self.finish_setting_change(Kind::Notice, &notice, vec![]);
        } else {
            match self.switch_model(model, effort, selected) {
                Ok(notice) => self.finish_setting_change(Kind::Notice, &notice, vec![]),
                Err(error) => self.finish_setting(Kind::Error, &error, vec![]),
            }
        }
    }

    /// Switches this conversation's model, reconnecting when `selected` is
    /// another provider or account; returns the change as its notice reads.
    pub(super) fn switch_model(
        &mut self,
        model: String,
        effort: Effort,
        selected: Option<backend::Services>,
    ) -> Result<String, String> {
        let agent = self.agent.as_mut().unwrap();
        let previous_provider = agent.backend_id();
        let previous_model = agent.model().to_owned();
        let previous_effort = agent.effort();
        let switch = selected.as_ref().is_some_and(|services| {
            agent.needs_configuration()
                || services.id() != agent.backend_id()
                || services.account() != agent.services().account()
        });
        if switch {
            let mut client = selected.unwrap().create(model.clone())?;
            client.set_effort(effort);
            agent.replace_backend(client)?;
        } else {
            agent.set_model(model.clone())?;
        }
        agent.set_effort(effort);
        let changed = agent.backend_id() != previous_provider;
        let notice = format!(
            "Model: {} → {} · this conversation",
            held::selection(
                changed.then_some(previous_provider),
                &previous_model,
                previous_effort
            ),
            held::selection(changed.then_some(agent.backend_id()), &model, effort)
        );
        self.archive = agent.archive();
        self.state.effort = effort.name().into();
        self.refresh_context();
        Ok(notice)
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
                                            self.facts().effort
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
        // Credential removal can finish after a question closed its panel.
        // Keep that completion from restoring a menu beneath the question.
        if self.state.ask_panel.is_some() {
            self.make_room_for_ask();
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
                option.fact = "the provider chooses · /effort NAME requests a level".into();
            }
        }
        self.state.selector = Some(menu);
    }
}
