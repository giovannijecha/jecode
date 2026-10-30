use super::Agent;
use crate::{
    backend::{Backend, Services},
    conversation::Origin,
    json::Value,
    sessions::Stage,
};

impl Agent {
    pub fn has_model(&self) -> bool {
        self.client.has_model()
    }

    pub fn needs_configuration(&self) -> bool {
        self.client.needs_configuration()
    }

    pub fn backend_id(&self) -> &'static str {
        self.client.id()
    }

    pub fn has_backend(&self, backend: &str) -> bool {
        self.accounts.contains_key(backend)
    }

    pub fn identity(&self) -> Origin {
        Origin::with_account(self.backend_id(), self.model(), self.client.account())
    }

    pub fn services(&self) -> Services {
        if self.client.needs_configuration()
            && let Some(services) = self.accounts.get(self.backend_id())
        {
            return services.clone();
        }
        self.client.services()
    }

    pub fn register_backend(&mut self, services: Services) {
        self.include_credentials(services.redactor());
        if services.is_configured() {
            self.accounts.insert(services.id().into(), services);
        }
    }

    pub fn include_credentials(&mut self, redactor: crate::redact::Redactor) {
        self.redactor.include(redactor);
        self.tools.include_credentials(self.redactor.clone());
    }

    /// Commits a prepared model connection while retaining this conversation's
    /// tools, processes, attachments and complete history.
    pub fn replace_backend(&mut self, next: Box<dyn Backend>) -> Result<(), String> {
        self.install_backend(next, true)
    }

    pub(super) fn install_backend(
        &mut self,
        mut next: Box<dyn Backend>,
        prepare: bool,
    ) -> Result<(), String> {
        let previous_identity = self.identity();
        let had_model = self.has_model();
        let identity = Origin::with_account(next.id(), next.model(), next.account());
        let changed = previous_identity != identity;
        let changed_effort = self.effort() != next.effort();
        // Remember replacement credentials before preparation or persistence
        // can produce diagnostics or append newly received history.
        self.redactor.include(next.redactor());
        self.tools.include_credentials(self.redactor.clone());
        next.set_cancellation(self.cancellation.clone());
        next.set_delegation(!self.child);
        if previous_identity.backend == identity.backend
            && previous_identity.model == identity.model
        {
            next.clone_routing_from(self.client.as_ref());
        } else {
            let preference = self.routing_preferences.as_ref().map(|preferences| {
                preferences
                    .get(&(identity.backend.clone(), identity.model.clone()))
                    .cloned()
                    .unwrap_or(crate::backend::Routing::Auto)
            });
            if let Some(preference) = preference {
                next.set_routing(preference);
            }
        }
        next.set_conversation(self.persistence.as_ref().map(crate::sessions::Handle::id));
        if prepare {
            next.prepare().map_err(|error| self.redact(&error))?;
        }
        let previous_context = self.context.clone();
        let previous_boundary = self.compatible_from;
        let mut recorded = None;
        if changed {
            self.compatible_from = self.messages.lock().unwrap().len();
            self.context.reset_usage();
            if had_model {
                let event = Value::object([
                    ("type", Value::string("backend_changed")),
                    ("from_backend", Value::string(previous_identity.backend)),
                    ("from_model", Value::string(previous_identity.model)),
                    ("backend", Value::string(&identity.backend)),
                    ("model", Value::string(&identity.model)),
                    ("after_message", Value::number(self.compatible_from)),
                ]);
                self.events.lock().unwrap().push(event.clone());
                recorded = Some(event);
            }
        }
        if changed || changed_effort {
            self.context.summary_output_tokens = 0;
        }
        let previous = std::mem::replace(&mut self.client, next);
        if let Err(error) = self.checkpoint(Stage::Selection) {
            self.client = previous;
            self.context = previous_context;
            self.compatible_from = previous_boundary;
            if let Some(event) = &recorded {
                self.withdraw_event(event);
            }
            return Err(error);
        }
        self.register_backend(self.client.services());
        Ok(())
    }
}
