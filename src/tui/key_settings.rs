use super::{App, Kind, jobs::Job, selector::Selector};
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
        let services = if self.agent.as_ref().unwrap().backend_id() == "openrouter" {
            self.agent
                .as_ref()
                .unwrap()
                .services()
                .with_credential(key.clone())
        } else {
            backend::Services::for_backend("openrouter", key.clone())
        };
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
        let agent = self.agent.as_mut().unwrap();
        let unconfigured = agent.needs_configuration();
        let reuse_model = agent.backend_id() == services.id() && agent.has_model();
        if reuse_model {
            let mut client = services.create(agent.model().into())?;
            client.set_effort(agent.effort());
            agent.replace_backend(client)?;
        } else {
            agent.register_backend(services.clone());
        }
        self.config.settings = settings;
        self.archive = agent.archive();
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
}
