use super::{App, Kind, held::Deferred, jobs::Job, selector::Selector};
use crate::backend;

impl App {
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
        let facts = self.facts();
        let services = if facts.backend_id == "openrouter" {
            facts.services.with_credential(key.clone())
        } else {
            backend::Services::for_backend("openrouter", key.clone())
        };
        drop(facts);
        match services {
            Err(error) => self.warn(&error),
            Ok(services) => {
                self.state.clear_notice();
                self.state.selector = Some(Selector::loading("API key", "Checking API key…", true));
                self.job = Some(Job::key(services, key));
            }
        }
    }

    pub(super) fn finish_key(
        &mut self,
        key: String,
        result: Result<backend::Services, String>,
    ) -> Result<(), String> {
        let services = match result {
            Ok(services) => services,
            Err(error) => {
                self.finish_setting(Kind::Error, &error, vec![]);
                return Ok(());
            }
        };
        let mut settings = self.config.settings.clone();
        settings
            .credentials
            .insert("openrouter".into(), key.clone());
        if settings.backend_id == "openrouter" {
            settings.api_key = key.clone();
        }
        let first_setup = !settings.has_model();
        if !first_setup && let Err(error) = self.config.store.save(&settings) {
            self.finish_setting(Kind::Error, &error, vec![]);
            return Ok(());
        }
        let facts = self.facts();
        let unconfigured = facts.needs_configuration;
        let reuse_model = facts.backend_id == services.id() && facts.has_model;
        drop(facts);
        if self.agent.is_some() {
            self.adopt_key(services.clone())?;
        } else {
            self.defer(Deferred::Key(services.clone()));
        }
        self.config.settings = settings;
        self.catalog.clear();
        self.refresh_context();
        self.finish_setting(
            Kind::Notice,
            if first_setup {
                "API key validated · Choose a default model to save configuration"
            } else {
                "API key validated and saved"
            },
            vec![],
        );
        if unconfigured && !reuse_model {
            self.offer_connected_models(services);
        }
        Ok(())
    }

    /// Reconnects the conversation's model with a validated key, or keeps the
    /// key for a later switch to its provider.
    pub(super) fn adopt_key(&mut self, services: backend::Services) -> Result<(), String> {
        let agent = self.agent.as_mut().unwrap();
        if agent.backend_id() == services.id() && agent.has_model() {
            let mut client = services.create(agent.model().into())?;
            client.set_effort(agent.effort());
            agent.replace_backend(client)?;
        } else {
            agent.register_backend(services);
        }
        self.archive = agent.archive();
        Ok(())
    }
}
