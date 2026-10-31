use super::{
    App, Feedback, Kind,
    held::Deferred,
    jobs::Job,
    selector::{Purpose, Selector},
};
use crate::auth::{AccountStore, Credential, SignInUpdate};
use crate::backend::Services;
use std::{path::PathBuf, sync::mpsc};

const USAGE_URL: &str = "https://chatgpt.com/settings/usage";

impl App {
    pub(super) fn sync_account_credentials(
        &mut self,
        provider: &'static str,
    ) -> Result<(), String> {
        if self.agent.is_none() {
            self.defer(Deferred::Credentials(provider));
            return Ok(());
        }
        let store = AccountStore::new(self.home(), provider);
        let redactor = store.redactor()?;
        let services = store
            .active()?
            .map(|session| Services::for_credential(&Credential::account(session)))
            .transpose()?;
        let agent = self.agent.as_mut().unwrap();
        agent.include_credentials(redactor);
        if let Some(services) = services {
            agent.register_backend(services);
        }
        self.archive = agent.archive();
        Ok(())
    }

    pub(super) fn provider_account_detail(&self, services: &Services) -> Option<String> {
        if !matches!(services.id(), "openai" | "anthropic") {
            return None;
        }
        let identity = services.account()?;
        let provider = services.id();
        let (plan, name) = if provider == "anthropic" {
            ("Claude", "Claude")
        } else {
            ("ChatGPT", "ChatGPT")
        };
        match AccountStore::new(self.home(), provider).accounts() {
            Ok(accounts) => Some(
                accounts
                    .into_iter()
                    .find(|account| account.identity == identity)
                    .map_or_else(
                        || format!("{name} account unavailable"),
                        |account| {
                            let detail = if account.detail.is_empty() {
                                &account.label
                            } else {
                                &account.detail
                            };
                            if account.enabled {
                                format!("Using {plan} plan · {detail}")
                            } else {
                                format!("{name} account signed out · {detail}")
                            }
                        },
                    ),
            ),
            Err(error) => Some(format!("{name} account unavailable · {error}")),
        }
    }

    fn home(&self) -> PathBuf {
        self.config
            .store
            .path()
            .parent()
            .expect("configuration directory")
            .to_path_buf()
    }

    pub(super) fn settings_menu(&self) -> Selector {
        let settings = &self.config.settings;
        Selector::settings(
            &settings.model,
            settings.effort,
            settings.auto_compact,
            &settings.routing_for(&settings.model),
            settings.default_permissions,
            self.config.store.path(),
        )
    }

    pub(super) fn models_menu(&self, defaults: bool) -> Selector {
        let facts = self.facts();
        let current = if defaults {
            &self.config.settings.model
        } else {
            &facts.model
        };
        let mut menu = Selector::models(&self.catalog, defaults, current);
        // The account the catalog came from rides the rule title.
        if let Some(detail) = self
            .catalog_services
            .as_ref()
            .and_then(|services| self.provider_account_detail(services))
        {
            menu.context = format!("{} · {detail}", menu.context);
        }
        if self.provider_parent.is_some() {
            menu.parent = Some("Provider");
        }
        menu
    }

    pub(super) fn open_settings(&mut self) {
        self.state.selector = Some(self.settings_menu());
    }

    pub(super) fn open_model_providers(&mut self, defaults: bool) {
        self.provider_parent = None;
        let mut services = self.config.settings.configured_services(&self.home());
        let facts = self.facts().into_owned();
        let active = facts.services;
        match services.len() {
            0 => self.finish_setting(
                Kind::Error,
                "No configured provider. Open Settings › Providers.",
                vec![],
            ),
            1 => self.load_catalog_for(
                super::jobs::CatalogPurpose::Models { defaults, id: None },
                if !facts.needs_configuration
                    && services[0].id() == active.id()
                    && services[0].account() == active.account()
                {
                    active
                } else {
                    services.remove(0)
                },
            ),
            _ => {
                let mut menu = Selector::providers(
                    &services,
                    defaults,
                    facts.backend_id,
                    &self.config.settings.backend_id,
                );
                for (option, service) in menu.options.iter_mut().zip(&services) {
                    if let Some(detail) = self.provider_account_detail(service) {
                        option.fact = detail;
                        if active.id() == service.id()
                            && active.account() != service.account()
                            && let Some(current) = self.provider_account_detail(&active)
                        {
                            option
                                .fact
                                .push_str(&format!(" · this conversation: {current}"));
                        }
                    }
                }
                self.state.selector = Some(menu);
            }
        }
    }

    pub(super) fn open_credentials(&mut self) {
        let openai = self
            .config
            .settings
            .services(&self.home(), "openai")
            .is_ok();
        let anthropic = self
            .config
            .settings
            .services(&self.home(), "anthropic")
            .is_ok();
        self.state.selector = Some(Selector::credentials(openai, anthropic));
    }

    pub(super) fn open_api_keys(&mut self) {
        let configured = self
            .config
            .settings
            .credential_for("openrouter")
            .is_some_and(|key| !key.is_empty());
        self.state.selector = Some(Selector::api_keys(configured));
    }

    pub(super) fn open_accounts(&mut self) {
        let store = AccountStore::new(self.home(), self.auth_provider);
        match store.accounts().and_then(|accounts| {
            let active = store
                .active()?
                .map(|session| session.account())
                .transpose()?
                .map(|account| account.id);
            Ok((accounts, active))
        }) {
            Ok((accounts, active)) => {
                self.auth_accounts = accounts;
                self.auth_account_index = None;
                self.state.selector = Some(Selector::accounts(
                    self.auth_provider,
                    &self.auth_accounts,
                    active.as_deref(),
                ));
            }
            Err(error) => {
                self.open_credentials();
                self.warn(&error);
            }
        }
    }

    pub(super) fn open_accounts_for(&mut self, provider: &'static str) {
        self.auth_provider = provider;
        self.open_accounts();
    }

    pub(super) fn open_account(&mut self, index: usize) {
        if let Some(account) = self.auth_accounts.get(index) {
            let active = match AccountStore::new(self.home(), self.auth_provider)
                .active()
                .and_then(|session| session.map(|session| session.account()).transpose())
            {
                Ok(current) => current.is_some_and(|current| current.id == account.id),
                Err(error) => {
                    self.warn(&error);
                    return;
                }
            };
            self.auth_account_index = Some(index);
            self.state.selector = Some(Selector::account(self.auth_provider, account, active));
        }
    }

    pub(super) fn activate_account(&mut self) {
        let Some(index) = self.auth_account_index else {
            return;
        };
        let Some(account) = self.auth_accounts.get(index).cloned() else {
            return;
        };
        let detail = if account.detail.is_empty() {
            account.label.clone()
        } else {
            account.detail.clone()
        };
        if !account.enabled {
            self.start_sign_in(Some(account.id.clone()));
            return;
        }
        let store = AccountStore::new(self.home(), self.auth_provider);
        match store
            .activate(&account.id)
            .and_then(|()| self.sync_account_credentials(self.auth_provider))
        {
            Ok(()) => {
                let facts = self.facts();
                let switch = facts.backend_id == self.auth_provider
                    && facts.services.account() != Some(account.identity);
                drop(facts);
                self.open_accounts();
                let provider_name = if self.auth_provider == "anthropic" {
                    "Anthropic"
                } else {
                    "OpenAI"
                };
                let plan_name = if self.auth_provider == "anthropic" {
                    "Claude"
                } else {
                    "ChatGPT"
                };
                self.state.notify(Feedback::result(
                    Kind::Notice,
                    format!(
                        "{provider_name} account selected · {detail} · Using your {plan_name} plan"
                    ),
                ));
                if switch {
                    match self
                        .config
                        .settings
                        .services(&self.home(), self.auth_provider)
                    {
                        Ok(services) => {
                            self.auth_model_switch = true;
                            self.load_catalog_for(
                                super::jobs::CatalogPurpose::Models {
                                    defaults: false,
                                    id: None,
                                },
                                services,
                            )
                        }
                        Err(error) => self.warn(&error),
                    }
                }
            }
            Err(error) => self.warn(&error),
        }
    }

    pub(super) fn confirm_sign_out(&mut self) {
        if let Some(menu) = &mut self.state.selector {
            menu.purpose = Purpose::Account {
                confirm_sign_out: true,
            };
        }
    }

    pub(super) fn start_sign_out(&mut self) {
        let Some(index) = self.auth_account_index else {
            return;
        };
        let Some(account) = self.auth_accounts.get(index) else {
            return;
        };
        let account_id = account.id.clone();
        self.state.selector = Some(Selector::loading(
            "Sign out",
            "Removing saved credentials…",
            false,
        ));
        self.auth_signing_out = true;
        self.job = Some(Job::sign_out(self.home(), self.auth_provider, account_id));
    }

    pub(super) fn finish_sign_out(&mut self, result: Result<Option<String>, String>) {
        self.auth_signing_out = false;
        self.open_accounts();
        match result {
            Ok(warning) => {
                self.refresh_context();
                let mut notice = "Signed out · saved credentials removed".to_string();
                if let Some(warning) = warning {
                    notice.push_str(&format!(" · {warning}"));
                }
                self.state.notify(Feedback::result(Kind::Notice, notice));
            }
            Err(error) => self.warn(&error),
        }
    }

    pub(super) fn start_sign_in(&mut self, existing: Option<String>) {
        let (sender, receiver) = mpsc::channel();
        self.auth_updates = Some(receiver);
        self.auth_browser_url = None;
        let (title, parent) = if self.auth_provider == "anthropic" {
            ("Continue with Claude", "Anthropic")
        } else {
            ("Continue with ChatGPT", "OpenAI")
        };
        let mut menu = Selector::loading(title, "Starting sign-in…", false);
        menu.parent = Some(parent);
        self.state.selector = Some(menu);
        self.job = Some(Job::sign_in(
            self.home(),
            self.auth_provider,
            existing,
            sender,
        ));
    }

    pub(super) fn poll_auth_updates(&mut self) -> bool {
        let mut dirty = false;
        let updates: Vec<_> = self
            .auth_updates
            .as_ref()
            .map(|receiver| receiver.try_iter().collect())
            .unwrap_or_default();
        for update in updates {
            if let Some(menu) = &mut self.state.selector {
                match update {
                    SignInUpdate::Browser(url) => {
                        self.auth_browser_url = Some(url.clone());
                        menu.message = format!("Open in browser: {url}");
                    }
                    SignInUpdate::Waiting => {
                        let plan = if self.auth_provider == "anthropic" {
                            "Claude"
                        } else {
                            "ChatGPT"
                        };
                        menu.message = match &self.auth_browser_url {
                            Some(url) => {
                                format!("Waiting for {plan} sign-in… · Open in browser: {url}")
                            }
                            None => format!("Waiting for {plan} sign-in… · Esc cancels"),
                        }
                    }
                }
                dirty = true;
            }
        }
        dirty
    }

    pub(super) fn finish_sign_in(
        &mut self,
        provider: &'static str,
        result: Result<crate::auth::Account, String>,
    ) {
        self.auth_updates = None;
        self.auth_browser_url = None;
        let result = result.and_then(|account| {
            if account.enabled {
                AccountStore::new(self.home(), provider)
                    .activate(&account.id)
                    .map(|()| true)
            } else {
                Ok(false)
            }
        });
        let result =
            result.and_then(|enabled| self.sync_account_credentials(provider).map(|()| enabled));
        if self.auth_provider == provider {
            self.open_accounts();
        }
        match result {
            Ok(true) => {
                let notice = if provider == "anthropic" {
                    "Connected Anthropic · Using your Claude plan"
                } else {
                    "Connected OpenAI · Using your ChatGPT plan · Manage usage in account details"
                };
                self.state.notify(Feedback::result(
                    Kind::Notice,
                    notice,
                ));
                if let Ok(services) = self.config.settings.services(&self.home(), provider) {
                    self.offer_connected_models(services);
                }
            }
            Ok(false) => self.state.notify(Feedback::result(Kind::Warning,
                if provider == "anthropic" {
                    "Signed in, but Claude plan access was not granted · check your plan or sign in again"
                } else {
                    "Signed in, but ChatGPT plan access was not granted · check your plan or sign in again"
                },
            )),
            Err(error) => self.warn(&error),
        }
    }

    pub(super) fn offer_connected_models(&mut self, services: Services) {
        let facts = self.facts();
        let offer = facts.needs_configuration
            || (facts.backend_id == services.id()
                && services.account() != facts.services.account());
        drop(facts);
        if offer {
            self.auth_model_switch = matches!(services.id(), "openai" | "anthropic");
            self.load_catalog_for(
                super::jobs::CatalogPurpose::Models {
                    defaults: !self.config.settings.has_model(),
                    id: None,
                },
                services,
            );
        }
    }

    pub(super) fn manage_usage(&mut self) {
        match crate::auth::open_browser(USAGE_URL) {
            Ok(()) => self.state.notify(Feedback::result(
                Kind::Notice,
                format!("Manage usage · {USAGE_URL}"),
            )),
            Err(error) => self.warn(&format!("{error}: {USAGE_URL}")),
        }
    }

    pub(super) fn close_provider_panel(&mut self) -> bool {
        let Some(menu) = self.state.selector.as_ref() else {
            return false;
        };
        match &menu.purpose {
            Purpose::Account {
                confirm_sign_out: true,
            } => {
                if let Some(menu) = &mut self.state.selector {
                    menu.purpose = Purpose::Account {
                        confirm_sign_out: false,
                    };
                }
            }
            Purpose::Account { .. } => self.open_accounts(),
            Purpose::Accounts => self.open_credentials(),
            Purpose::Credentials | Purpose::ApiKeys => {
                self.state.selector = Some(Selector::providers_settings())
            }
            Purpose::ProviderSettings => self.return_to_settings(),
            Purpose::Key => self.open_api_keys(),
            Purpose::Loading if self.auth_signing_out => return true,
            Purpose::Loading if self.auth_model_switch => {
                drop(self.job.take());
                self.auth_model_switch = false;
                self.open_accounts();
            }
            Purpose::Models { .. } if self.auth_model_switch => {
                self.auth_model_switch = false;
                self.open_accounts();
            }
            Purpose::Efforts { .. } if self.auth_model_switch => {
                self.state.selector = Some(self.models_menu(false));
            }
            Purpose::Providers { .. } => {
                if self.settings_parent.is_some() {
                    self.return_to_settings();
                } else {
                    self.state.selector = None;
                }
            }
            Purpose::Efforts { defaults, .. } if self.provider_parent.is_some() => {
                self.state.selector = Some(self.models_menu(*defaults));
            }
            Purpose::Models { .. } if self.provider_parent.is_some() => {
                let defaults = self.provider_parent.take().unwrap();
                self.open_model_providers(defaults);
            }
            Purpose::Loading if self.auth_updates.is_some() => {
                self.auth_updates = None;
                self.auth_browser_url = None;
                drop(self.job.take());
                self.open_accounts();
                self.state
                    .notify(Feedback::result(Kind::Warning, "Sign-in cancelled"));
            }
            _ => return false,
        }
        true
    }
}
