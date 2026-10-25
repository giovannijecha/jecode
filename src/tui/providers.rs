use super::{
    App, Feedback, Kind,
    jobs::Job,
    selector::{Purpose, Selector},
};
use crate::auth::openai::{SignInUpdate, Store};
use crate::backend::Services;
use std::{path::PathBuf, process::Command, sync::mpsc};

const USAGE_URL: &str = "https://chatgpt.com/settings/usage";

impl App {
    fn sync_openai_credentials(&mut self) -> Result<(), String> {
        let store = Store::new(self.home());
        let redactor = store.redactor()?;
        let services = store
            .active()?
            .map(|session| {
                crate::backend::Services::for_credential(&crate::auth::Credential::openai(session))
            })
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
        if services.id() != "openai" {
            return None;
        }
        let identity = services.account()?;
        match Store::new(self.home()).accounts() {
            Ok(accounts) => Some(
                accounts
                    .into_iter()
                    .find(|account| {
                        format!("{}:{}", account.client_id, account.subject) == identity
                    })
                    .map_or_else(
                        || "ChatGPT account unavailable".to_string(),
                        |account| {
                            if account.enabled {
                                format!("Using ChatGPT plan · {}", account.email)
                            } else {
                                format!("ChatGPT account signed out · {}", account.email)
                            }
                        },
                    ),
            ),
            Err(error) => Some(format!("ChatGPT account unavailable · {error}")),
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
            self.config.store.path(),
        )
    }

    pub(super) fn models_menu(&self, defaults: bool) -> Selector {
        let mut menu = Selector::models(
            &self.catalog,
            defaults,
            if defaults {
                &self.config.settings.model
            } else {
                &self.archive.model
            },
        );
        if let Some(detail) = self
            .catalog_services
            .as_ref()
            .and_then(|services| self.provider_account_detail(services))
        {
            for option in &mut menu.options {
                option.prose = detail.clone();
            }
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
        let active = self.agent.as_ref().unwrap().services();
        match services.len() {
            0 => self.finish_setting(
                Kind::Error,
                "No configured provider. Open Settings › Providers.",
                vec![],
            ),
            1 => self.load_catalog_for(
                super::jobs::CatalogPurpose::Models { defaults, id: None },
                if services[0].id() == active.id() && services[0].account() == active.account() {
                    active
                } else {
                    services.remove(0)
                },
            ),
            _ => {
                let mut menu = Selector::providers(
                    &services,
                    defaults,
                    self.agent.as_ref().unwrap().backend_id(),
                    &self.config.settings.backend_id,
                );
                for (option, service) in menu.options.iter_mut().zip(&services) {
                    if let Some(detail) = self.provider_account_detail(service) {
                        option.prose = detail;
                        if active.id() == "openai"
                            && active.account() != service.account()
                            && let Some(current) = self.provider_account_detail(&active)
                        {
                            option
                                .prose
                                .push_str(&format!(" · Current conversation: {current}"));
                        }
                    }
                }
                self.state.selector = Some(menu);
            }
        }
    }

    pub(super) fn open_credentials(&mut self) {
        let configured = self
            .config
            .settings
            .services(&self.home(), "openai")
            .is_ok();
        self.state.selector = Some(Selector::credentials(configured));
    }

    pub(super) fn open_api_keys(&mut self) {
        let configured = self.config.settings.credential_for("openrouter").is_some();
        self.state.selector = Some(Selector::api_keys(configured));
    }

    pub(super) fn open_accounts(&mut self) {
        let store = Store::new(self.home());
        match store.accounts().and_then(|accounts| {
            let active = store
                .active()?
                .map(|session| session.account())
                .transpose()?
                .map(|account| account.client_id);
            Ok((accounts, active))
        }) {
            Ok((accounts, active)) => {
                self.auth_accounts = accounts;
                self.auth_account_index = None;
                self.state.selector =
                    Some(Selector::accounts(&self.auth_accounts, active.as_deref()));
            }
            Err(error) => {
                self.state.selector = Some(Selector::credentials(false));
                self.warn(&error);
            }
        }
    }

    pub(super) fn open_account(&mut self, index: usize) {
        if let Some(account) = self.auth_accounts.get(index) {
            let active = match Store::new(self.home())
                .active()
                .and_then(|session| session.map(|session| session.account()).transpose())
            {
                Ok(current) => {
                    current.is_some_and(|current| current.client_id == account.client_id)
                }
                Err(error) => {
                    self.warn(&error);
                    return;
                }
            };
            self.auth_account_index = Some(index);
            self.state.selector = Some(Selector::account(account, active));
        }
    }

    pub(super) fn activate_account(&mut self) {
        let Some(index) = self.auth_account_index else {
            return;
        };
        let Some(account) = self.auth_accounts.get(index).cloned() else {
            return;
        };
        let email = account.email.clone();
        if !account.enabled {
            self.start_sign_in(Some(account.client_id.clone()));
            return;
        }
        let store = Store::new(self.home());
        match store
            .activate(&account.client_id)
            .and_then(|()| self.sync_openai_credentials())
        {
            Ok(()) => {
                let switch = self.agent.as_ref().unwrap().backend_id() == "openai"
                    && self.agent.as_ref().unwrap().services().account()
                        != Some(format!("{}:{}", account.client_id, account.subject));
                self.open_accounts();
                self.state.notify(Feedback::result(
                    Kind::Notice,
                    format!("OpenAI account selected · {email} · Using your ChatGPT plan"),
                ));
                if switch {
                    match self.config.settings.services(&self.home(), "openai") {
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
        let subject = account.client_id.clone();
        self.state.selector = Some(Selector::loading(
            "Sign out",
            "Removing saved credentials…",
            false,
        ));
        self.auth_signing_out = true;
        self.job = Some(Job::sign_out(self.home(), subject));
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
        let mut menu = Selector::loading("Continue with ChatGPT", "Starting sign-in…", false);
        menu.parent = Some("OpenAI");
        self.state.selector = Some(menu);
        self.job = Some(Job::sign_in(self.home(), existing, sender));
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
                        menu.message = match &self.auth_browser_url {
                            Some(url) => {
                                format!("Waiting for ChatGPT sign-in… · Open in browser: {url}")
                            }
                            None => "Waiting for ChatGPT sign-in… · Esc cancels".into(),
                        }
                    }
                }
                dirty = true;
            }
        }
        dirty
    }

    pub(super) fn finish_sign_in(&mut self, result: Result<crate::auth::openai::Account, String>) {
        self.auth_updates = None;
        self.auth_browser_url = None;
        let result = result.and_then(|account| {
            if account.enabled {
                Store::new(self.home())
                    .activate(&account.client_id)
                    .map(|()| true)
            } else {
                Ok(false)
            }
        });
        let result = result.and_then(|enabled| self.sync_openai_credentials().map(|()| enabled));
        self.open_accounts();
        match result {
            Ok(true) => {
                self.state.notify(Feedback::result(
                    Kind::Notice,
                    "Connected OpenAI · Using your ChatGPT plan · Manage usage in account details",
                ));
                if self.agent.as_ref().unwrap().backend_id() == "openai"
                    && let Ok(services) = self.config.settings.services(&self.home(), "openai")
                    && services.account() != self.agent.as_ref().unwrap().services().account()
                {
                    self.auth_model_switch = true;
                    self.load_catalog_for(
                        super::jobs::CatalogPurpose::Models {
                            defaults: false,
                            id: None,
                        },
                        services,
                    );
                }
            }
            Ok(false) => self.state.notify(Feedback::result(
                Kind::Warning,
                "Signed in, but ChatGPT plan access was not granted · check your plan or sign in again",
            )),
            Err(error) => self.warn(&error),
        }
    }

    pub(super) fn manage_usage(&mut self) {
        match open_external(USAGE_URL) {
            Ok(()) => self.state.notify(Feedback::result(
                Kind::Notice,
                format!("Manage usage · {USAGE_URL}"),
            )),
            Err(error) => self.warn(&error),
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

fn open_external(url: &str) -> Result<(), String> {
    if !url.starts_with("https://") || url.chars().any(char::is_whitespace) {
        return Err("Unsafe sign-in link".into());
    }
    #[cfg(target_os = "windows")]
    let program = "explorer.exe";
    #[cfg(target_os = "macos")]
    let program = "open";
    #[cfg(all(not(target_os = "windows"), not(target_os = "macos")))]
    let program = "xdg-open";
    Command::new(program)
        .arg(url)
        .spawn()
        .map(|_| ())
        .map_err(|error| format!("Open this link in a browser: {url} · {error}"))
}
