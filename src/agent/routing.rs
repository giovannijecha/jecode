use super::Agent;
use crate::backend::{Charges, Completion, Endpoints, Routing};
use std::collections::BTreeMap;

impl Agent {
    pub fn routing(&self) -> Routing {
        self.client.routing()
    }

    pub fn cached_endpoints(&self, model: &str) -> Option<Endpoints> {
        self.client
            .cached_endpoints()
            .filter(|cached| cached.model == model)
    }

    /// Applies from this conversation's next request; saved defaults are unchanged.
    pub fn set_routing(&mut self, routing: Routing) {
        self.client.set_routing(routing);
    }

    /// Saved per-model preferences, applied when the model changes and to new
    /// conversations. Without them, as in isolated tests, routing stays unset.
    #[cfg(test)]
    pub fn set_routing_preferences(&mut self, preferences: BTreeMap<String, Routing>) {
        let backend = self.backend_id().to_owned();
        self.set_backend_routing_preferences(&backend, preferences);
    }

    pub fn set_backend_routing_preferences(
        &mut self,
        backend: &str,
        preferences: BTreeMap<String, Routing>,
    ) {
        let saved = self.routing_preferences.get_or_insert_with(BTreeMap::new);
        saved.retain(|(id, _), _| id != backend);
        saved.extend(
            preferences
                .into_iter()
                .map(|(model, routing)| ((backend.into(), model), routing)),
        );
    }

    /// The saved preference for `model`; Auto when none is saved.
    #[cfg(test)]
    pub(super) fn preferred_routing(&self, model: &str) -> Routing {
        match &self.routing_preferences {
            Some(preferences) => preferences
                .get(&(self.backend_id().into(), model.into()))
                .cloned()
                .unwrap_or(Routing::Auto),
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
        if let Some(charges) = &completion.charges {
            self.charges.lock().unwrap().include(charges);
        }
    }

    /// Prepares routing for a turn: clears the previous turn's charges and
    /// refreshes stale endpoint metadata, at most once per freshness window.
    pub(super) fn prepare_routing(&mut self) -> Result<(), String> {
        *self.charges.lock().unwrap() = Charges::default();
        self.client.prepare()?;
        if self.cancellation.requested() {
            return Err("Operation cancelled".into());
        }
        Ok(())
    }

    /// Auto's `session_id` follows the saved conversation it belongs to.
    pub(super) fn sync_conversation(&mut self) {
        let id = self.persistence.as_ref().map(crate::sessions::Handle::id);
        self.client.set_conversation(id);
    }
}
