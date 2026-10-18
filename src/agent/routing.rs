use super::Agent;
use crate::openrouter::{Charges, Completion, Endpoints, Router, Routing};
use std::collections::BTreeMap;

impl Agent {
    pub fn routing(&self) -> &Routing {
        self.client.routing()
    }

    pub fn router(&self) -> &Router {
        self.client.router()
    }

    /// Applies from this conversation's next request; saved defaults are unchanged.
    pub fn set_routing(&mut self, routing: Routing) {
        self.client.set_routing(routing);
    }

    /// Saved per-model preferences, applied when the model changes and to new
    /// conversations. Without them, as in isolated tests, routing stays unset.
    pub fn set_routing_preferences(&mut self, preferences: BTreeMap<String, Routing>) {
        self.routing_preferences = Some(preferences);
    }

    /// The saved preference for `model`; Auto when none is saved.
    pub(super) fn preferred_routing(&self, model: &str) -> Routing {
        match &self.routing_preferences {
            Some(preferences) => preferences.get(model).cloned().unwrap_or(Routing::Auto),
            None => Routing::Default,
        }
    }

    /// Endpoint metadata fetched elsewhere; ignored when it is for another model.
    pub fn offer_endpoints(&mut self, endpoints: Endpoints) {
        self.client.offer_endpoints(endpoints);
    }

    /// What responses reported for the latest turn's requests.
    pub fn charges(&self) -> Charges {
        self.charges.lock().unwrap().clone()
    }

    pub(super) fn record_charges(&self, completion: &Completion) {
        self.charges
            .lock()
            .unwrap()
            .record(completion.provider.as_deref(), completion.usage.as_ref());
    }

    /// Prepares routing for a turn: clears the previous turn's charges and
    /// refreshes stale endpoint metadata, at most once per freshness window.
    pub(super) fn prepare_routing(&mut self) -> Result<(), String> {
        *self.charges.lock().unwrap() = Charges::default();
        self.client.refresh_endpoints();
        if self.cancellation.requested() {
            return Err("Operation cancelled".into());
        }
        self.client.router().validate_pin(
            self.model(),
            self.effort().reasons(),
            std::time::Instant::now(),
        )
    }

    /// Auto's `session_id` follows the saved conversation it belongs to.
    pub(super) fn sync_conversation(&mut self) {
        let id = self.persistence.as_ref().map(crate::sessions::Handle::id);
        self.client.set_conversation(id);
    }
}
