use super::{
    App, Kind,
    jobs::{Job, RoutingPurpose},
    selector::{Purpose, Selector},
};
use crate::backend::{Endpoints, Services};

impl App {
    /// Opens routing for the conversation or, with `defaults`, the default
    /// model's saved routing. A fresh listing opens at once; otherwise it is
    /// fetched in the background and Esc cancels the request.
    pub(super) fn open_routing(&mut self, defaults: bool) {
        let agent = self.agent.as_ref().unwrap();
        let model = if defaults {
            self.config.settings.model.clone()
        } else {
            agent.model().to_owned()
        };
        let purpose = RoutingPurpose { defaults, model };
        if (!defaults || agent.backend_id() == self.config.settings.backend_id)
            && let Some(endpoints) = agent.cached_endpoints(&purpose.model)
        {
            return self.show_routing(purpose, Ok(Some(endpoints)));
        }
        let services = match self.routing_services(defaults) {
            Ok(services) => services,
            Err(error) => return self.finish_setting(Kind::Error, &error, vec![]),
        };
        let title = if defaults {
            "Default routing"
        } else {
            "Routing"
        };
        let mut menu = Selector::loading(title, "Loading endpoints…", defaults);
        menu.context.clone_from(&purpose.model);
        self.state.selector = Some(menu);
        self.state.clear_notice();
        self.job = Some(Job::endpoints(services, purpose));
    }

    /// Shows the choices once a listing arrives or fails. Without a listing,
    /// Auto and the current choice remain available.
    pub(super) fn show_routing(
        &mut self,
        purpose: RoutingPurpose,
        result: Result<Option<Endpoints>, String>,
    ) {
        let RoutingPurpose { defaults, model } = purpose;
        let agent = self.agent.as_mut().unwrap();
        let target = if defaults {
            self.config.settings.model.as_str()
        } else {
            agent.model()
        };
        if model != target {
            return self.finish_setting(
                Kind::Warning,
                &format!("Endpoints for {model} discarded · the model changed meanwhile"),
                vec![],
            );
        }
        let (current, reasoning) = if defaults {
            let settings = &self.config.settings;
            (settings.routing_for(&model), settings.effort.reasons())
        } else {
            (agent.routing().clone(), agent.effort().reasons())
        };
        let mut notes = Vec::new();
        let endpoints = match result {
            Ok(Some(endpoints)) => {
                // Ignored unless the conversation uses this model.
                if !defaults || agent.backend_id() == self.config.settings.backend_id {
                    agent.offer_endpoints(endpoints.clone());
                }
                Some(endpoints)
            }
            Ok(None) => {
                return self.finish_setting(
                    Kind::Error,
                    "Endpoint routing is unavailable for this backend",
                    vec![],
                );
            }
            Err(error) => {
                notes.push(format!("Endpoint listing unavailable · {error}"));
                None
            }
        };
        if !defaults && let Some(served) = agent.charges().describe() {
            notes.push(format!("Last turn · {served}"));
        }
        let notes = notes
            .iter()
            .map(|note| self.archive.redactor.text(note))
            .collect();
        self.state.clear_progress();
        let services = match self.routing_services(defaults) {
            Ok(services) => services,
            Err(error) => return self.finish_setting(Kind::Error, &error, vec![]),
        };
        let Some(entries) = services.routing_entries(endpoints.as_ref(), &current, reasoning)
        else {
            return self.finish_setting(
                Kind::Error,
                "Endpoint routing is unavailable for this backend",
                vec![],
            );
        };
        self.state.selector = Some(Selector::routing(
            &model, defaults, entries, &current, notes,
        ));
    }

    fn routing_services(&self, defaults: bool) -> Result<Services, String> {
        let agent = self.agent.as_ref().unwrap();
        let id = &self.config.settings.backend_id;
        if !defaults || id == agent.backend_id() {
            return Ok(agent.services());
        }
        let key = self
            .config
            .settings
            .credential_for(id)
            .ok_or_else(|| format!("{id} has no saved credentials"))?;
        Services::for_backend(id, key.into())
    }

    /// Applies a chosen entry: to the conversation from its next request, or
    /// saved as the model's default for new conversations and model switches.
    pub(super) fn choose_routing(&mut self, menu: Selector, index: usize) {
        let Purpose::Routing {
            defaults,
            model,
            entries,
            ..
        } = &menu.purpose
        else {
            return;
        };
        let (defaults, model, entry) = (*defaults, model.clone(), entries[index].clone());
        if let Some(problem) = &entry.problem {
            self.state.selector = Some(menu);
            return self.warn(&format!("{problem} · routing unchanged"));
        }
        if defaults {
            let mut settings = self.config.settings.clone();
            let previous = settings.routing_for(&model);
            settings
                .routing
                .insert(model.clone(), entry.routing.clone());
            match self.config.store.save(&settings) {
                Ok(()) => {
                    self.config.settings = settings;
                    self.agent
                        .as_mut()
                        .unwrap()
                        .set_backend_routing_preferences(
                            &self.config.settings.backend_id,
                            self.config.settings.routing.clone(),
                        );
                    self.finish_setting_change(
                        Kind::Notice,
                        &format!(
                            "Default routing for {model}: {} → {} · new conversations and model switches",
                            previous.label(),
                            entry.routing.label()
                        ),
                        vec![],
                    );
                }
                Err(error) => self.finish_setting(Kind::Error, &error, vec![]),
            }
            return;
        }
        let agent = self.agent.as_mut().unwrap();
        if agent.model() != model {
            return self.finish_setting(
                Kind::Warning,
                &format!("Routing for {model} discarded · the model changed meanwhile"),
                vec![],
            );
        }
        let previous = agent.routing().label();
        agent.set_routing(entry.routing.clone());
        self.finish_setting_change(
            Kind::Notice,
            &format!(
                "Routing: {previous} → {} · from the next request",
                entry.routing.label()
            ),
            vec![],
        );
    }
}
