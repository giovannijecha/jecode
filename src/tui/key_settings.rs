use super::{
    App, Kind,
    held::Deferred,
    jobs::Job,
    selector::{Purpose, Selector},
};
use crate::backend;

impl App {
    pub(super) fn save_key(&mut self) {
        let Some(menu) = self.state.selector.as_ref() else {
            return;
        };
        let Purpose::Key(id) = &menu.purpose else {
            return;
        };
        let id = *id;
        let title = menu.title.clone();
        let key = menu.editor.text.trim().to_string();
        let facts = self.facts();
        let services = if facts.backend_id == id {
            facts.services.with_credential(key.clone())
        } else {
            backend::Services::for_backend(id, key.clone())
        };
        drop(facts);
        match services {
            Err(error) => self.warn(&error),
            Ok(services) => {
                self.state.clear_notice();
                self.state.selector = Some(Selector::loading(&title, "Checking API key…", true));
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
            .insert(services.id().into(), key.clone());
        if settings.backend_id == services.id() {
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

    /// Removes a saved key through the same store as `jecode providers
    /// logout`. The running conversation keeps the connection it has.
    pub(super) fn remove_key(&mut self, id: &'static str) {
        let label = backend::info(id).map_or_else(|| id.to_owned(), |info| info.key_label());
        let mut settings = self.config.settings.clone();
        settings.credentials.remove(id);
        if settings.backend_id == id {
            settings.api_key.clear();
        }
        // Without a saved model the key lived only in this session.
        let removed = if self.config.settings.has_model() {
            self.config.store.remove_credential(id)
        } else {
            Ok(true)
        };
        let removed = match removed {
            Ok(removed) => removed,
            Err(error) => {
                self.finish_setting(Kind::Error, &error, vec![]);
                return;
            }
        };
        self.config.settings = settings;
        self.catalog.clear();
        let text = if removed {
            format!("{label} removed · this conversation keeps its connection")
        } else {
            // Nothing on disk: the key came from the environment or another edit.
            format!("{label} cleared for this session · config.json had no saved copy")
        };
        self.finish_setting_change(Kind::Notice, &text, vec![]);
        self.open_api_keys();
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
