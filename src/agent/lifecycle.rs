use super::Agent;
use crate::{
    backend::{Backend, Charges, Routing},
    effort::Effort,
    sessions::{Document, Handle, Input},
};

impl Agent {
    fn connection(
        &self,
        backend: &str,
        model: String,
        effort: Effort,
        routing: Routing,
    ) -> Result<Box<dyn Backend>, String> {
        let services = self
            .accounts
            .get(backend)
            .ok_or_else(|| format!("Backend {backend} has no configured account"))?;
        let mut client = services.create(model)?;
        if backend == self.backend_id() && client.model() == self.model() {
            client.clone_routing_from(self.client.as_ref());
        }
        client.set_effort(effort);
        client.set_routing(routing);
        client.set_cancellation(self.cancellation.clone());
        Ok(client)
    }

    fn selection_routing(&self, backend: &str, model: &str) -> Routing {
        match &self.routing_preferences {
            Some(preferences) => preferences
                .get(&(backend.into(), model.into()))
                .cloned()
                .unwrap_or(Routing::Auto),
            None => Routing::Default,
        }
    }

    #[cfg(test)]
    pub fn start_new(&mut self, model: String, effort: Effort) -> Result<(), String> {
        self.start_new_selection(self.backend_id(), model, effort)
    }

    /// New conversations use the selected account; live resources end with the old one.
    pub fn start_new_selection(
        &mut self,
        backend: &str,
        model: String,
        effort: Effort,
    ) -> Result<(), String> {
        let routing = self.selection_routing(backend, &model);
        let client = self.connection(backend, model.clone(), effort, routing.clone())?;
        self.save_session()?;
        let next = if let Some(old) = &self.persistence {
            let input = old.snapshot().input;
            let mut document = Document::new(self.tools.root().to_path_buf(), model, effort)?;
            document.backend = backend.into();
            document.routing = routing;
            document.input = input.clone();
            document.messages = vec![self.system_message()];
            let next = Handle::new(old.store(), document, self.redactor.clone());
            next.flush()?;
            old.input(Input {
                history: input.history.clone(),
                ..Input::default()
            });
            if let Err(error) = old.flush() {
                old.input(input);
                return Err(error);
            }
            Some(next)
        } else {
            None
        };
        let temporary = next
            .as_ref()
            .map(|handle| handle.store().temporary_area(&handle.id()))
            .transpose()?;
        self.client = client;
        self.redactor.include(self.client.redactor());
        self.tools.include_credentials(self.redactor.clone());
        self.tools.stop_processes();
        self.clear();
        self.persistence = next;
        self.sync_conversation();
        self.begin_checkpoints();
        if let Some(handle) = &self.persistence {
            self.tools.configure_session_output(
                handle.store().output_directory(),
                &handle.id(),
                self.redactor.clone(),
            )?;
        }
        if let Some(temporary) = temporary {
            self.tools.configure_temporary(temporary);
        }
        Ok(())
    }

    pub fn resume(&mut self, id: &str) -> Result<String, String> {
        let current = self
            .persistence
            .as_ref()
            .ok_or("Session storage is unavailable")?;
        if current.id() == id {
            return Err("This session is already open".into());
        }
        self.save_session()?;
        let (handle, status) = Handle::open(current.store(), id, self.redactor.clone())?;
        let document = handle.snapshot();
        let unselected = document.model.is_none();
        if unselected {
            // Bind only the saved draft, before changing the live agent. A
            // failed selection save must leave the current conversation intact.
            let mut archive = self.archive();
            archive.messages =
                std::sync::Arc::new(std::sync::Mutex::new(document.messages.clone()));
            archive.events = std::sync::Arc::new(std::sync::Mutex::new(document.events.clone()));
            archive.legacy_origin = document.legacy_origin.clone();
            handle.checkpoint(
                &archive,
                document.compatible_from,
                &document.context,
                &self.routing(),
                self.has_model(),
                crate::sessions::Stage::Selection,
            )?;
        }
        let client = document
            .model
            .as_ref()
            .map(|model| {
                self.connection(
                    &document.backend,
                    model.clone(),
                    document.effort,
                    document.routing.clone(),
                )
            })
            .transpose()?;
        let document = handle.snapshot();
        let temporary = handle.store().temporary_area(&document.id)?;
        if let Some(client) = client {
            self.client = client;
        }
        self.redactor.include(self.client.redactor());
        self.tools.include_credentials(self.redactor.clone());
        *self.charges.lock().unwrap() = Charges::default();
        self.tools.stop_processes();
        *self.messages.lock().unwrap() = document.messages;
        self.rebuild_used_skills();
        *self.events.lock().unwrap() = document.events;
        self.compatible_from = document.compatible_from;
        self.legacy_origin = document.legacy_origin;
        self.context = document.context;
        if self.context.from > 1 && self.context.skill_references.is_empty() {
            self.refresh_skills();
            self.context.skill_references = self.redact(&self.skill_checkpoint_instructions());
        }
        let calibration = self.context.calibration;
        self.context.reset_usage();
        self.context.calibration = calibration;
        self.prepared = None;
        self.project_instructions.clear();
        self.cancellation.reset();
        self.persistence = Some(handle);
        self.sync_conversation();
        self.begin_checkpoints();
        let handle = self.persistence.as_ref().unwrap();
        self.tools.configure_session_output(
            handle.store().output_directory(),
            &handle.id(),
            self.redactor.clone(),
        )?;
        self.tools.configure_temporary(temporary);
        Ok(status)
    }

    #[cfg(test)]
    pub(crate) fn delete_session(
        &mut self,
        id: &str,
        model: String,
        effort: Effort,
    ) -> Result<(crate::sessions::DeleteReport, bool), String> {
        self.delete_session_selection(id, self.backend_id(), model, effort)
    }

    pub(crate) fn delete_session_selection(
        &mut self,
        id: &str,
        backend: &str,
        model: String,
        effort: Effort,
    ) -> Result<(crate::sessions::DeleteReport, bool), String> {
        let current = self
            .persistence
            .as_ref()
            .ok_or("Session storage is unavailable")?;
        if current.id() != id {
            return current.store().delete(id).map(|report| (report, false));
        }
        if self.prepared.is_some() || current.active() {
            return Err("Session deletion is available only when the turn is ready".into());
        }
        let routing = self.selection_routing(backend, &model);
        let client = self.connection(backend, model.clone(), effort, routing.clone())?;
        let store = current.store();
        let mut document = Document::new(self.tools.root().to_path_buf(), model, effort)?;
        document.backend = backend.into();
        document.routing = routing;
        document.input = current.snapshot().input;
        document.input.history.clear();
        let temporary = store.temporary_area(&document.id)?;
        let outputs = crate::output::Store::for_session(
            store.output_directory(),
            self.tools.root().to_path_buf(),
            &document.id,
            self.redactor.clone(),
        )?;
        let next = Handle::new(store, document, self.redactor.clone());
        let live_attachments = next.snapshot().input.attachment_ids();
        self.tools.stop_browsers();
        let report = current.delete(&live_attachments, || self.tools.stop_processes())?;
        self.client = client;
        self.redactor.include(self.client.redactor());
        self.tools.include_credentials(self.redactor.clone());
        self.clear();
        self.cancellation.reset();
        self.persistence = Some(next);
        self.sync_conversation();
        self.begin_checkpoints();
        self.tools.configure_output_store(outputs);
        self.tools.configure_temporary(temporary);
        Ok((report, true))
    }
}
