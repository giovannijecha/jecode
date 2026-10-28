use super::Agent;
use crate::{
    json::Value,
    sessions::{Document, Handle, Stage, Store},
};
use std::path::Path;

impl Agent {
    pub fn enable_sessions(&mut self, home: &Path) -> Result<(), String> {
        if self.persistence.is_none() {
            self.skills.configure(home);
            self.tools.configure_browser_profiles(home);
            self.tools.configure_project_map(home);
            let store = Store::new(home.to_path_buf(), self.tools.root())?;
            let mut document = Document::new(
                self.tools.root().to_path_buf(),
                if self.has_model() {
                    self.model().into()
                } else {
                    String::new()
                },
                self.effort(),
            )?;
            document.routing = self.routing().clone();
            document.backend = self.backend_id().into();
            self.tools.configure_session_output(
                store.output_directory(),
                &document.id,
                self.redactor.clone(),
            )?;
            self.tools
                .configure_temporary(store.temporary_area(&document.id)?);
            self.tools.configure_attachments(store.attachments());
            self.context.reset_usage();
            self.persistence = Some(Handle::new(store, document, self.redactor.clone()));
            self.sync_conversation();
            self.collaboration
                .bind(self.archive(), self.persistence.clone())?;
            self.begin_checkpoints();
            self.checkpoint(Stage::Preserve)?;
        }
        Ok(())
    }

    /// Copies files into the session attachment pool, all or none.
    pub fn import_attachments(
        &self,
        paths: &[std::path::PathBuf],
    ) -> Result<Vec<crate::attachments::Attachment>, String> {
        let pool = self
            .tools
            .attachments()
            .ok_or("Attachments need session storage, which is unavailable")?;
        paths
            .iter()
            .map(|path| pool.import_file(path, &self.cancellation))
            .collect()
    }

    pub fn sessions(&self) -> Option<Handle> {
        self.persistence.clone()
    }

    pub(super) fn checkpoint(&self, stage: Stage) -> Result<(), String> {
        match &self.persistence {
            Some(handle) => handle.checkpoint(
                &self.archive(),
                self.compatible_from,
                &self.context,
                &self.routing(),
                self.has_model(),
                stage,
            ),
            None => Ok(()),
        }
    }

    pub fn save_session(&self) -> Result<(), String> {
        self.checkpoint(Stage::Preserve)
    }

    pub fn prepare_turn(
        &mut self,
        prompt: impl Into<crate::attachments::Prompt>,
    ) -> Result<(), String> {
        let prompt = prompt.into();
        if prompt.is_empty() {
            return Err("The prompt must not be empty".into());
        }
        if self.prepared.is_some() {
            return Err("A prompt is already prepared".into());
        }
        if self.needs_configuration() {
            self.client.prepare()?;
        }
        self.refresh_project_instructions()?;
        self.refresh_skills();
        let before = self.messages.lock().unwrap().len();
        let previous_events = self.events.lock().unwrap().len();
        self.events.lock().unwrap().push(Value::object([
            ("type", Value::string("turn_start")),
            ("after_message", Value::number(before)),
        ]));
        self.messages.lock().unwrap().push(prompt.message());
        if let Err(error) = self.checkpoint(Stage::Begin) {
            self.messages.lock().unwrap().truncate(before);
            self.events.lock().unwrap().truncate(previous_events);
            return Err(error);
        }
        self.prepared = Some(prompt);
        Ok(())
    }

    pub(super) fn record_turn_error(&self, error: &str, partial: &str) {
        let after_message = self.messages.lock().unwrap().len();
        self.events.lock().unwrap().push(Value::object([
            ("type", Value::string("turn_error")),
            ("error", Value::string(self.redact(error))),
            (
                "kind",
                Value::string(if self.cancellation.requested() {
                    "warning"
                } else {
                    "error"
                }),
            ),
            ("partial_text", Value::string(partial)),
            ("after_message", Value::number(after_message)),
        ]));
    }
}
