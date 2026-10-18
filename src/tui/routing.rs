use super::{
    App, Kind,
    jobs::{Job, RoutingPurpose},
    selector::{Purpose, Selector},
};
use crate::openrouter::{Endpoints, routing};
use std::time::Instant;

/// How to read the endpoint rows.
pub(super) const LEGEND: &str =
    "$/M input/output after discounts · 30-minute medians for speed and latency";

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
        let fresh = agent
            .router()
            .endpoints(&purpose.model)
            .filter(|endpoints| endpoints.fresh(Instant::now()))
            .cloned();
        match fresh {
            Some(endpoints) => self.show_routing(purpose, Ok(endpoints)),
            None => {
                let mut menu = Selector::loading();
                menu.title = "Loading endpoints…".into();
                self.state.selector = Some(menu);
                self.state.clear_notice();
                self.job = Some(Job::endpoints(agent.api(), purpose));
            }
        }
    }

    /// Shows the choices once a listing arrives or fails. Without a listing,
    /// Auto and the current choice remain available.
    pub(super) fn show_routing(
        &mut self,
        purpose: RoutingPurpose,
        result: Result<Endpoints, String>,
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
            Ok(endpoints) => {
                // Ignored unless the conversation uses this model.
                agent.offer_endpoints(endpoints.clone());
                notes.push(LEGEND.to_owned());
                Some(endpoints)
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
        let entries = routing::entries(endpoints.as_ref(), &current, reasoning);
        self.state.selector = Some(Selector::routing(
            &model, defaults, entries, &current, notes,
        ));
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
                        .set_routing_preferences(self.config.settings.routing.clone());
                    self.finish_setting(
                        Kind::Notice,
                        &format!(
                            "Default routing for {model}: {} · was {} · applies to new conversations and model switches",
                            entry.routing.label(),
                            previous.label()
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
        self.finish_setting(
            Kind::Notice,
            &format!(
                "Routing set to {} · was {previous} · applies from the next request",
                entry.routing.label()
            ),
            vec![],
        );
    }
}
