//! While a turn holds the agent, menus read what they show from a snapshot
//! taken when the turn started, and the choices made meanwhile wait here
//! until the turn returns the agent.

use super::{App, Kind};
use crate::{
    agent::Agent,
    backend::{Endpoints, Routing, Services},
    effort::Effort,
};
use std::{borrow::Cow, collections::BTreeMap, path::PathBuf};

/// What the menus read about the conversation's agent.
#[derive(Clone)]
pub(super) struct Facts {
    pub backend_id: &'static str,
    pub services: Services,
    pub needs_configuration: bool,
    pub has_model: bool,
    pub model: String,
    pub effort: Effort,
    /// A live preference shared with the execution worker and its collaborators.
    pub auto_compact: std::sync::Arc<std::sync::atomic::AtomicU8>,
    pub routing: Routing,
    routing_preferences: Option<BTreeMap<(String, String), Routing>>,
    /// The listing cached for `model`.
    pub endpoints: Option<Endpoints>,
    /// Where the last turn was served.
    pub charges: Option<String>,
    /// The user skill library's directory.
    pub skills: Option<PathBuf>,
}

impl Facts {
    fn of(agent: &Agent) -> Self {
        Self {
            backend_id: agent.backend_id(),
            services: agent.services(),
            needs_configuration: agent.needs_configuration(),
            has_model: agent.has_model(),
            model: agent.model().into(),
            effort: agent.effort(),
            auto_compact: agent.auto_compact_preference(),
            routing: agent.routing(),
            routing_preferences: agent.routing_preferences_snapshot(),
            endpoints: agent.cached_endpoints(agent.model()),
            charges: agent.charges().describe(),
            skills: agent.skill_directory(),
        }
    }

    fn cached_endpoints(&self, model: &str) -> Option<Endpoints> {
        self.endpoints.clone().filter(|_| self.model == model)
    }

    fn selection_routing(&self, backend: &str, model: &str) -> Routing {
        self.routing_preferences
            .as_ref()
            .map_or(Routing::Default, |preferences| {
                preferences
                    .get(&(backend.into(), model.into()))
                    .cloned()
                    .unwrap_or(Routing::Auto)
            })
    }
}

/// A choice made during a turn, applied in order once the agent returns.
pub(super) enum Deferred {
    Model {
        model: String,
        effort: Effort,
        services: Option<Services>,
    },
    Routing {
        model: String,
        routing: Routing,
    },
    /// The saved routing preferences changed.
    DefaultRouting {
        backend: String,
        preferences: BTreeMap<String, Routing>,
    },
    Register(Services),
    /// The active ChatGPT account changed.
    Credentials,
    Key(Services),
}

impl App {
    /// Takes the agent for a turn, keeping what the menus read meanwhile.
    pub(super) fn hold(&mut self) -> Agent {
        let agent = self.agent.take().expect("agent available");
        self.held = Some(Facts::of(&agent));
        agent
    }

    pub(super) fn facts(&self) -> Cow<'_, Facts> {
        match (&self.agent, &self.held) {
            (Some(agent), _) => Cow::Owned(Facts::of(agent)),
            (None, Some(held)) => Cow::Borrowed(held),
            (None, None) => unreachable!("menus open while the agent or its snapshot is here"),
        }
    }

    /// Waits for the turn, or applies at once when the agent is here.
    pub(super) fn defer(&mut self, change: Deferred) {
        if let Some(held) = &mut self.held {
            // Later menus show the conversation as it will be.
            match &change {
                Deferred::Model {
                    model,
                    effort,
                    services,
                } => {
                    let replacing = services.as_ref().is_some_and(|services| {
                        held.needs_configuration
                            || services.id() != held.backend_id
                            || services.account() != held.services.account()
                    });
                    if replacing || *model != held.model {
                        let backend = services.as_ref().map_or(held.backend_id, Services::id);
                        held.routing = held.selection_routing(backend, model);
                        held.endpoints = None;
                    }
                    if let Some(services) = services {
                        held.backend_id = services.id();
                        held.services = services.clone();
                        held.needs_configuration = false;
                    }
                    model.clone_into(&mut held.model);
                    held.effort = *effort;
                }
                Deferred::Routing { routing, .. } => held.routing = routing.clone(),
                Deferred::DefaultRouting {
                    backend,
                    preferences,
                } => {
                    let saved = held.routing_preferences.get_or_insert_with(BTreeMap::new);
                    saved.retain(|(id, _), _| id != backend);
                    saved.extend(preferences.iter().map(|(model, routing)| {
                        ((backend.clone(), model.clone()), routing.clone())
                    }));
                }
                Deferred::Key(services) if held.backend_id == services.id() => {
                    held.services = services.clone();
                }
                _ => {}
            }
        }
        self.deferred.push(change);
        if self.agent.is_some() {
            self.apply_deferred();
        }
    }

    /// Applies the choices made during the turn that just returned the agent.
    pub(super) fn apply_deferred(&mut self) {
        self.held = None;
        for change in std::mem::take(&mut self.deferred) {
            match change {
                Deferred::Model {
                    model,
                    effort,
                    services,
                } => {
                    let result = self.switch_model(model, effort, services);
                    self.record("/model", result, Kind::Error);
                }
                Deferred::Routing { model, routing } => {
                    let result = self.switch_routing(&model, routing);
                    self.record("/routing", result, Kind::Warning);
                }
                Deferred::DefaultRouting {
                    backend,
                    preferences,
                } => {
                    self.agent
                        .as_mut()
                        .unwrap()
                        .set_backend_routing_preferences(&backend, preferences);
                }
                Deferred::Register(services) => {
                    let agent = self.agent.as_mut().unwrap();
                    agent.register_backend(services);
                    self.archive = agent.archive();
                }
                Deferred::Credentials => {
                    if let Err(error) = self.sync_account_credentials() {
                        self.warn(&error);
                    }
                }
                Deferred::Key(services) => {
                    if let Err(error) = self.adopt_key(services) {
                        self.record("/settings", Err(error), Kind::Error);
                    }
                }
            }
        }
    }

    /// A conversation change applied after the turn, as its own command row.
    fn record(&mut self, command: &str, result: Result<String, String>, failure: Kind) {
        self.local_start(command);
        match result {
            Ok(message) => self.local_finish_change(Kind::Notice, &message, vec![]),
            Err(error) => self.local_finish(failure, &error, vec![]),
        }
    }

    pub(super) fn cached_endpoints(&self, model: &str) -> Option<Endpoints> {
        match (&self.agent, &self.held) {
            (Some(agent), _) => agent.cached_endpoints(model),
            (None, held) => held.as_ref()?.cached_endpoints(model),
        }
    }
}

/// `openai / gpt-5 (high)` when the provider changes, `gpt-5 (high)` otherwise.
pub(super) fn selection(provider: Option<&str>, model: &str, effort: Effort) -> String {
    match provider {
        Some(provider) => format!("{provider} / {model} ({})", effort.name()),
        None => format!("{model} ({})", effort.name()),
    }
}
