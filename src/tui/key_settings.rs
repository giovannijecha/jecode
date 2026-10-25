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
        result: Result<(), String>,
    ) -> Result<(), String> {
        if let Err(error) = result {
            self.finish_setting(Kind::Error, &error, vec![]);
            return Ok(());
        }
        let mut settings = self.config.settings.clone();
        settings
            .credentials
            .insert("openrouter".into(), key.clone());
        if settings.backend_id == "openrouter" {
            settings.api_key = key.clone();
        }
        if let Err(error) = self.config.store.save(&settings) {
            self.finish_setting(Kind::Error, &error, vec![]);
            return Ok(());
        }
        let agent = self.agent.as_mut().unwrap();
        let choose_model = agent.model() == "unconfigured";
        if agent.backend_id() == "openrouter" && !choose_model {
            let mut client = agent
                .services()
                .with_credential(key.clone())?
                .create(agent.model().into())?;
            client.set_effort(agent.effort());
            agent.replace_backend(client)?;
        } else {
            agent.register_backend(backend::Services::for_backend("openrouter", key)?);
        }
        self.config.settings = settings;
        self.archive = agent.archive();
        self.catalog.clear();
        self.refresh_context();
        self.finish_setting(Kind::Notice, "API key validated and saved", vec![]);
        if choose_model {
            self.open_model_providers(false);
        }
        Ok(())
    }
}
